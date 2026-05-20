use serde::Deserialize;
use serde_json::{Value, json};
use std::fmt;
use std::io;
use std::net::TcpStream;
use std::thread;
use std::time::Duration;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket, connect};

const CDP_IO_TIMEOUT: Duration = Duration::from_secs(15);
const READY_STATE_ATTEMPTS: usize = 50;
const READY_STATE_POLL: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub(crate) enum CdpError {
    Base64(base64::DecodeError),
    Http(ureq::Error),
    Io(io::Error),
    Json(serde_json::Error),
    WebSocket(tungstenite::Error),
    NoPageTarget,
    ProtocolError(Value),
    ReadyStateTimeout,
    UnexpectedResponse(Value),
}

impl fmt::Display for CdpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CdpError::Http(error) => write!(f, "{error}"),
            CdpError::Base64(error) => write!(f, "{error}"),
            CdpError::Io(error) => write!(f, "{error}"),
            CdpError::Json(error) => write!(f, "{error}"),
            CdpError::WebSocket(error) => write!(f, "{error}"),
            CdpError::NoPageTarget => write!(f, "could not find a CDP page target"),
            CdpError::ProtocolError(value) => write!(f, "CDP returned error: {value}"),
            CdpError::ReadyStateTimeout => {
                write!(f, "timed out waiting for page readiness")
            }
            CdpError::UnexpectedResponse(value) => write!(f, "unexpected CDP response: {value}"),
        }
    }
}

impl From<base64::DecodeError> for CdpError {
    fn from(error: base64::DecodeError) -> Self {
        CdpError::Base64(error)
    }
}

impl From<ureq::Error> for CdpError {
    fn from(error: ureq::Error) -> Self {
        CdpError::Http(error)
    }
}

impl From<io::Error> for CdpError {
    fn from(error: io::Error) -> Self {
        CdpError::Io(error)
    }
}

impl From<serde_json::Error> for CdpError {
    fn from(error: serde_json::Error) -> Self {
        CdpError::Json(error)
    }
}

impl From<tungstenite::Error> for CdpError {
    fn from(error: tungstenite::Error) -> Self {
        CdpError::WebSocket(error)
    }
}

pub(crate) fn page_target(port: u16) -> Result<Target, CdpError> {
    let url = format!("http://127.0.0.1:{port}/json/list");
    let mut response = ureq::get(&url).call()?;
    let targets: Vec<Target> = response.body_mut().read_json()?;

    targets
        .into_iter()
        .find(|target| target.kind == "page" && target.web_socket_debugger_url.starts_with("ws://"))
        .ok_or(CdpError::NoPageTarget)
}

pub(crate) struct CdpConnection {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: u64,
}

impl CdpConnection {
    pub(crate) fn connect(ws_url: &str) -> Result<Self, CdpError> {
        let (mut socket, _) = connect(ws_url)?;
        set_socket_timeout(&mut socket)?;

        Ok(Self { socket, next_id: 1 })
    }

    pub(crate) fn evaluate_json(&mut self, expression: &str) -> Result<Value, CdpError> {
        let response = self.call(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "awaitPromise": true,
                "returnByValue": true
            }),
        )?;

        response
            .get("result")
            .and_then(|result| result.get("value"))
            .cloned()
            .ok_or(CdpError::UnexpectedResponse(response))
    }

    pub(crate) fn navigate(&mut self, url: &str) -> Result<(), CdpError> {
        self.call("Page.navigate", json!({ "url": url }))?;
        self.wait_for_ready_state()
    }

    fn wait_for_ready_state(&mut self) -> Result<(), CdpError> {
        self.wait_for_ready_state_with(READY_STATE_ATTEMPTS, READY_STATE_POLL)
    }

    fn wait_for_ready_state_with(
        &mut self,
        attempts: usize,
        poll: Duration,
    ) -> Result<(), CdpError> {
        for _ in 0..attempts {
            let value = self.evaluate_json("document.readyState")?;

            if matches!(value.as_str(), Some("interactive" | "complete")) {
                return Ok(());
            }

            thread::sleep(poll);
        }

        Err(CdpError::ReadyStateTimeout)
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, CdpError> {
        let id = self.next_id;
        self.next_id += 1;

        let request = json!({
            "id": id,
            "method": method,
            "params": params
        });

        self.socket
            .send(Message::Text(request.to_string().into()))?;

        loop {
            let message = self.socket.read()?;
            let Message::Text(text) = message else {
                continue;
            };
            let response: Value = serde_json::from_str(&text)?;

            if response.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }

            if let Some(error) = response.get("error") {
                return Err(CdpError::ProtocolError(error.clone()));
            }

            return response
                .get("result")
                .cloned()
                .ok_or(CdpError::UnexpectedResponse(response));
        }
    }
}

fn set_socket_timeout(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>) -> Result<(), CdpError> {
    if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
        stream.set_read_timeout(Some(CDP_IO_TIMEOUT))?;
        stream.set_write_timeout(Some(CDP_IO_TIMEOUT))?;
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
pub(crate) struct Target {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    pub(crate) web_socket_debugger_url: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn cdp_connection_reports_protocol_error_packet() {
        let port = start_fake_cdp_server(FakeCdpMode::ProtocolError);
        let mut connection = connect_fake_cdp(port);

        let error = connection.evaluate_json("1 + 1").err().unwrap();

        assert!(matches!(error, CdpError::ProtocolError(_)));
        assert!(error.to_string().contains("fake CDP failure"));
    }

    #[test]
    fn cdp_ready_state_timeout_is_explicit() {
        let port = start_fake_cdp_server(FakeCdpMode::LoadingReadyState);
        let mut connection = connect_fake_cdp(port);

        let error = connection
            .wait_for_ready_state_with(1, Duration::ZERO)
            .err()
            .unwrap();

        assert!(matches!(error, CdpError::ReadyStateTimeout));
    }

    #[derive(Clone, Copy)]
    enum FakeCdpMode {
        ProtocolError,
        LoadingReadyState,
    }

    fn connect_fake_cdp(port: u16) -> CdpConnection {
        CdpConnection::connect(&format!("ws://127.0.0.1:{port}/devtools/page/1")).unwrap()
    }

    fn start_fake_cdp_server(mode: FakeCdpMode) -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();

        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();

            loop {
                let Ok(message) = socket.read() else {
                    break;
                };
                let Message::Text(text) = message else {
                    continue;
                };
                let request: Value = serde_json::from_str(&text).unwrap();
                let id = request.get("id").and_then(Value::as_u64).unwrap();

                match mode {
                    FakeCdpMode::ProtocolError => send_response(
                        &mut socket,
                        json!({
                            "id": id,
                            "error": {
                                "code": -32000,
                                "message": "fake CDP failure"
                            }
                        }),
                    ),
                    FakeCdpMode::LoadingReadyState => send_response(
                        &mut socket,
                        json!({
                            "id": id,
                            "result": {
                                "result": {
                                    "type": "string",
                                    "value": "loading"
                                }
                            }
                        }),
                    ),
                }
            }
        });

        port
    }

    fn send_response(socket: &mut tungstenite::WebSocket<TcpStream>, value: Value) {
        socket
            .send(Message::Text(value.to_string().into()))
            .unwrap();
    }
}
