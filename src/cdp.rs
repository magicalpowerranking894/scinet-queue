use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fmt;
use std::net::TcpStream;
use std::thread;
use std::time::Duration;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket, connect};

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionProbe {
    pub title: String,
    pub url: String,
    pub text: String,
}

impl SessionProbe {
    pub fn is_logged_in(&self) -> bool {
        let text = self.text.to_ascii_lowercase();

        text.contains("tokens") || text.contains("logout") || text.contains("library")
    }
}

#[derive(Debug)]
pub enum CdpError {
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
            CdpError::Json(error) => write!(f, "{error}"),
            CdpError::WebSocket(error) => write!(f, "{error}"),
            CdpError::NoPageTarget => write!(f, "could not find a CDP page target"),
            CdpError::UnexpectedResponse(value) => write!(f, "unexpected CDP response: {value}"),
        }
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

pub fn probe_session(port: u16, url: &str) -> Result<SessionProbe, CdpError> {
    let target = page_target(port)?;
    let mut cdp = CdpConnection::connect(&target.web_socket_debugger_url)?;

    cdp.call("Page.navigate", json!({ "url": url }))?;
    cdp.wait_for_ready_state()?;

    let value = cdp.evaluate_json(
        "({ title: document.title, url: location.href, text: (document.body && document.body.innerText || '').slice(0, 4000) })",
    )?;

    serde_json::from_value(value).map_err(CdpError::Json)
}

fn page_target(port: u16) -> Result<Target, CdpError> {
    let url = format!("http://127.0.0.1:{port}/json/list");
    let mut response = ureq::get(&url).call()?;
    let targets: Vec<Target> = response.body_mut().read_json()?;

    targets
        .into_iter()
        .find(|target| target.kind == "page" && target.web_socket_debugger_url.starts_with("ws://"))
        .ok_or(CdpError::NoPageTarget)
}

struct CdpConnection {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: u64,
}

impl CdpConnection {
    fn connect(ws_url: &str) -> Result<Self, CdpError> {
        let (socket, _) = connect(ws_url)?;

        Ok(Self { socket, next_id: 1 })
    }

    fn evaluate_json(&mut self, expression: &str) -> Result<Value, CdpError> {
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
struct Target {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    web_socket_debugger_url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_logged_in_session_text() {
        let probe = SessionProbe {
            title: "Sci-Net".to_string(),
            url: "https://sci-net.xyz/".to_string(),
            text: "user tokens library".to_string(),
        };

        assert!(probe.is_logged_in());
    }

    #[test]
    fn detects_logged_out_session_text() {
        let probe = SessionProbe {
            title: "Sci-Net".to_string(),
            url: "https://sci-net.xyz/".to_string(),
            text: "login sign up".to_string(),
        };

        assert!(!probe.is_logged_in());
    }
}
