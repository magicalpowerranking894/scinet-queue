use serde_json::Value;
use std::fmt;

use crate::cdp::{CdpConnection, CdpError, page_target};

pub(crate) trait PageSession {
    fn navigate(&mut self, url: &str) -> Result<(), PageError>;
    fn evaluate_json(&mut self, expression: &str) -> Result<Value, PageError>;
}

pub(crate) struct CdpPageSession {
    connection: CdpConnection,
}

impl CdpPageSession {
    pub(crate) fn connect(port: u16) -> Result<Self, PageError> {
        let target = page_target(port)?;
        let connection = CdpConnection::connect(&target.web_socket_debugger_url)?;

        Ok(Self { connection })
    }
}

impl PageSession for CdpPageSession {
    fn navigate(&mut self, url: &str) -> Result<(), PageError> {
        self.connection.navigate(url).map_err(PageError::Cdp)
    }

    fn evaluate_json(&mut self, expression: &str) -> Result<Value, PageError> {
        self.connection
            .evaluate_json(expression)
            .map_err(PageError::Cdp)
    }
}

#[derive(Debug)]
pub(crate) enum PageError {
    Base64(base64::DecodeError),
    Cdp(CdpError),
    Json(serde_json::Error),
    UnexpectedResponse(Value),
}

impl fmt::Display for PageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PageError::Base64(error) => write!(f, "{error}"),
            PageError::Cdp(error) => write!(f, "{error}"),
            PageError::Json(error) => write!(f, "{error}"),
            PageError::UnexpectedResponse(value) => {
                write!(f, "unexpected browser response: {value}")
            }
        }
    }
}

impl From<base64::DecodeError> for PageError {
    fn from(error: base64::DecodeError) -> Self {
        PageError::Base64(error)
    }
}

impl From<CdpError> for PageError {
    fn from(error: CdpError) -> Self {
        PageError::Cdp(error)
    }
}

impl From<serde_json::Error> for PageError {
    fn from(error: serde_json::Error) -> Self {
        PageError::Json(error)
    }
}
