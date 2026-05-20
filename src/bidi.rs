use serde_json::{Value, json};
use std::fmt;
use std::io;
use std::net::TcpStream;
use std::thread;
use std::time::Duration;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket, connect};

const BIDI_IO_TIMEOUT: Duration = Duration::from_secs(15);
const BIDI_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const BIDI_CONNECT_POLL: Duration = Duration::from_millis(50);
const READY_STATE_ATTEMPTS: usize = 50;
const READY_STATE_POLL: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub(crate) enum BidiError {
    Io(io::Error),
    Json(serde_json::Error),
    WebSocket(tungstenite::Error),
    ConnectTimeout(u16),
    ProtocolError(Value),
    ReadyStateTimeout,
    UnexpectedResponse(Value),
}

impl fmt::Display for BidiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BidiError::Io(error) => write!(f, "{error}"),
            BidiError::Json(error) => write!(f, "{error}"),
            BidiError::WebSocket(error) => write!(f, "{error}"),
            BidiError::ConnectTimeout(port) => {
                write!(f, "timed out connecting to BiDi on 127.0.0.1:{port}")
            }
            BidiError::ProtocolError(value) => write!(f, "BiDi returned error: {value}"),
            BidiError::ReadyStateTimeout => {
                write!(f, "timed out waiting for page readiness")
            }
            BidiError::UnexpectedResponse(value) => {
                write!(f, "unexpected BiDi response: {value}")
            }
        }
    }
}

impl From<io::Error> for BidiError {
    fn from(error: io::Error) -> Self {
        BidiError::Io(error)
    }
}

impl From<serde_json::Error> for BidiError {
    fn from(error: serde_json::Error) -> Self {
        BidiError::Json(error)
    }
}

impl From<tungstenite::Error> for BidiError {
    fn from(error: tungstenite::Error) -> Self {
        BidiError::WebSocket(error)
    }
}

pub(crate) struct BidiConnection {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: u64,
    context: String,
}

impl BidiConnection {
    pub(crate) fn connect(port: u16) -> Result<Self, BidiError> {
        Self::connect_with_timeout(port, BIDI_CONNECT_TIMEOUT)
    }

    fn connect_with_timeout(port: u16, timeout: Duration) -> Result<Self, BidiError> {
        let ws_url = format!("ws://127.0.0.1:{port}/session");
        let start = std::time::Instant::now();

        let mut socket = loop {
            match connect(&ws_url) {
                Ok((socket, _)) => break socket,
                Err(_) => {
                    if start.elapsed() >= timeout {
                        return Err(BidiError::ConnectTimeout(port));
                    }

                    thread::sleep(BIDI_CONNECT_POLL);
                }
            }
        };
        set_socket_timeout(&mut socket)?;

        let mut connection = Self {
            socket,
            next_id: 1,
            context: String::new(),
        };

        connection.call("session.new", json!({ "capabilities": {} }))?;
        connection.context = connection.create_context()?;

        Ok(connection)
    }

    pub(crate) fn navigate(&mut self, url: &str) -> Result<(), BidiError> {
        self.call(
            "browsingContext.navigate",
            json!({
                "context": self.context,
                "url": url,
                "wait": "complete"
            }),
        )?;
        self.wait_for_ready_state()
    }

    pub(crate) fn evaluate_json(&mut self, expression: &str) -> Result<Value, BidiError> {
        let wrapped = format!("(async () => JSON.stringify(await ({expression})))()");
        let response = self.call(
            "script.evaluate",
            json!({
                "expression": wrapped,
                "target": { "context": self.context },
                "awaitPromise": true
            }),
        )?;

        let value = response
            .get("result")
            .and_then(|result| result.get("value"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or(BidiError::UnexpectedResponse(response))?;

        serde_json::from_str(&value).map_err(BidiError::Json)
    }

    fn create_context(&mut self) -> Result<String, BidiError> {
        let response = self.call("browsingContext.create", json!({ "type": "tab" }))?;

        response
            .get("context")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or(BidiError::UnexpectedResponse(response))
    }

    fn wait_for_ready_state(&mut self) -> Result<(), BidiError> {
        self.wait_for_ready_state_with(READY_STATE_ATTEMPTS, READY_STATE_POLL)
    }

    fn wait_for_ready_state_with(
        &mut self,
        attempts: usize,
        poll: Duration,
    ) -> Result<(), BidiError> {
        for _ in 0..attempts {
            let value = self.evaluate_json("document.readyState")?;

            if matches!(value.as_str(), Some("interactive" | "complete")) {
                return Ok(());
            }

            thread::sleep(poll);
        }

        Err(BidiError::ReadyStateTimeout)
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, BidiError> {
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

            if response.get("type").and_then(Value::as_str) == Some("error") {
                return Err(BidiError::ProtocolError(response));
            }

            return response
                .get("result")
                .cloned()
                .ok_or(BidiError::UnexpectedResponse(response));
        }
    }
}

fn set_socket_timeout(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>) -> Result<(), BidiError> {
    if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
        stream.set_read_timeout(Some(BIDI_IO_TIMEOUT))?;
        stream.set_write_timeout(Some(BIDI_IO_TIMEOUT))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[test]
    fn bidi_connection_ignores_events_and_evaluates_json() {
        let methods = Arc::new(Mutex::new(Vec::new()));
        let port = start_fake_bidi_server(methods.clone(), FakeBidiMode::Ok);

        let mut connection =
            BidiConnection::connect_with_timeout(port, Duration::from_secs(2)).unwrap();
        connection.navigate("https://sci-net.xyz/").unwrap();
        let value = connection
            .evaluate_json("({ answer: 42, ok: true })")
            .unwrap();

        assert_eq!(value, json!({ "answer": 42, "ok": true }));
        assert_eq!(
            *methods.lock().unwrap(),
            vec![
                "session.new",
                "browsingContext.create",
                "browsingContext.navigate",
                "script.evaluate",
                "script.evaluate",
            ]
        );
    }

    #[test]
    fn bidi_connection_reports_error_response() {
        let methods = Arc::new(Mutex::new(Vec::new()));
        let port = start_fake_bidi_server(methods, FakeBidiMode::FailSession);

        let error = BidiConnection::connect_with_timeout(port, Duration::from_secs(2))
            .err()
            .unwrap();

        assert!(matches!(error, BidiError::ProtocolError(_)));
        assert!(error.to_string().contains("session not created"));
    }

    #[test]
    fn bidi_ready_state_timeout_is_explicit() {
        let methods = Arc::new(Mutex::new(Vec::new()));
        let port = start_fake_bidi_server(methods, FakeBidiMode::NeverReady);
        let mut connection =
            BidiConnection::connect_with_timeout(port, Duration::from_secs(2)).unwrap();

        let error = connection
            .wait_for_ready_state_with(1, Duration::ZERO)
            .err()
            .unwrap();

        assert!(matches!(error, BidiError::ReadyStateTimeout));
    }

    #[derive(Clone, Copy)]
    enum FakeBidiMode {
        Ok,
        FailSession,
        NeverReady,
    }

    fn start_fake_bidi_server(methods: Arc<Mutex<Vec<&'static str>>>, mode: FakeBidiMode) -> u16 {
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
                let method = request.get("method").and_then(Value::as_str).unwrap();

                match method {
                    "session.new" => {
                        methods.lock().unwrap().push("session.new");

                        if matches!(mode, FakeBidiMode::FailSession) {
                            send_response(
                                &mut socket,
                                json!({
                                    "id": id,
                                    "type": "error",
                                    "error": "session not created",
                                    "message": "session not created"
                                }),
                            );
                            continue;
                        }

                        send_response(
                            &mut socket,
                            json!({
                                "type": "event",
                                "method": "log.entryAdded",
                                "params": {}
                            }),
                        );
                        send_response(
                            &mut socket,
                            json!({
                                "id": id,
                                "type": "success",
                                "result": {}
                            }),
                        );
                    }
                    "browsingContext.create" => {
                        methods.lock().unwrap().push("browsingContext.create");
                        send_response(
                            &mut socket,
                            json!({
                                "id": id,
                                "type": "success",
                                "result": { "context": "ctx-1" }
                            }),
                        );
                    }
                    "browsingContext.navigate" => {
                        methods.lock().unwrap().push("browsingContext.navigate");
                        send_response(
                            &mut socket,
                            json!({
                                "id": id,
                                "type": "success",
                                "result": {}
                            }),
                        );
                    }
                    "script.evaluate" => {
                        methods.lock().unwrap().push("script.evaluate");
                        let expression = request
                            .get("params")
                            .and_then(|params| params.get("expression"))
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let value = if expression.contains("document.readyState") {
                            match mode {
                                FakeBidiMode::NeverReady => "\"loading\"",
                                FakeBidiMode::Ok | FakeBidiMode::FailSession => "\"complete\"",
                            }
                        } else {
                            "{\"answer\":42,\"ok\":true}"
                        };

                        send_response(
                            &mut socket,
                            json!({
                                "id": id,
                                "type": "success",
                                "result": {
                                    "realm": "realm-1",
                                    "result": {
                                        "type": "string",
                                        "value": value
                                    }
                                }
                            }),
                        );
                    }
                    _ => {
                        send_response(
                            &mut socket,
                            json!({
                                "id": id,
                                "type": "error",
                                "error": "unknown command",
                                "message": method
                            }),
                        );
                    }
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
