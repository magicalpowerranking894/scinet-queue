use serde_json::{Value, json};
use std::fmt;
use std::io;
use std::net::TcpStream;
use std::thread;
use std::time::Duration;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket, connect};

const BIDI_IO_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub(crate) enum BidiError {
    Io(io::Error),
    Json(serde_json::Error),
    WebSocket(tungstenite::Error),
    UnexpectedResponse(Value),
}

impl fmt::Display for BidiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BidiError::Io(error) => write!(f, "{error}"),
            BidiError::Json(error) => write!(f, "{error}"),
            BidiError::WebSocket(error) => write!(f, "{error}"),
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
        let ws_url = format!("ws://127.0.0.1:{port}/session");
        let (mut socket, _) = connect(&ws_url)?;
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
        for _ in 0..50 {
            let value = self.evaluate_json("document.readyState")?;

            if matches!(value.as_str(), Some("interactive" | "complete")) {
                return Ok(());
            }

            thread::sleep(Duration::from_millis(100));
        }

        Ok(())
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
                return Err(BidiError::UnexpectedResponse(response));
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
