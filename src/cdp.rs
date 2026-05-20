use serde::Deserialize;
use serde_json::{Value, json};
use std::fmt;
use std::net::TcpStream;
use std::thread;
use std::time::Duration;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket, connect};

#[derive(Debug)]
pub(crate) enum CdpError {
    Base64(base64::DecodeError),
    Http(ureq::Error),
    Json(serde_json::Error),
    WebSocket(tungstenite::Error),
    NoPageTarget,
    UnexpectedResponse(Value),
}

impl fmt::Display for CdpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CdpError::Http(error) => write!(f, "{error}"),
            CdpError::Base64(error) => write!(f, "{error}"),
            CdpError::Json(error) => write!(f, "{error}"),
            CdpError::WebSocket(error) => write!(f, "{error}"),
            CdpError::NoPageTarget => write!(f, "could not find a CDP page target"),
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
        let (socket, _) = connect(ws_url)?;

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
        for _ in 0..50 {
            let value = self.evaluate_json("document.readyState")?;

            if matches!(value.as_str(), Some("interactive" | "complete")) {
                return Ok(());
            }

            thread::sleep(Duration::from_millis(100));
        }

        Ok(())
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

            return response
                .get("result")
                .cloned()
                .ok_or(CdpError::UnexpectedResponse(response));
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct Target {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    pub(crate) web_socket_debugger_url: String,
}
