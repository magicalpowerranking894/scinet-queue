use serde_json::Value;
use std::fmt;

use crate::bidi::{BidiConnection, BidiError};
use crate::browser::BrowserEngine;
use crate::cdp::{CdpConnection, CdpError, page_target};

pub(crate) trait PageSession {
    fn navigate(&mut self, url: &str) -> Result<(), PageError>;
    fn evaluate_json(&mut self, expression: &str) -> Result<Value, PageError>;
}

pub(crate) struct CdpPageSession {
    connection: CdpConnection,
}

pub(crate) struct BidiPageSession {
    connection: BidiConnection,
}

pub(crate) enum BrowserPageSession {
    Cdp(CdpPageSession),
    Bidi(BidiPageSession),
}

pub(crate) fn connect_page_session(
    engine: BrowserEngine,
    port: u16,
) -> Result<BrowserPageSession, PageError> {
    match engine {
        BrowserEngine::Chromium => CdpPageSession::connect(port).map(BrowserPageSession::Cdp),
        BrowserEngine::Firefox => BidiPageSession::connect(port).map(BrowserPageSession::Bidi),
    }
}

impl CdpPageSession {
    pub(crate) fn connect(port: u16) -> Result<Self, PageError> {
        let target = page_target(port)?;
        let connection = CdpConnection::connect(&target.web_socket_debugger_url)?;

        Ok(Self { connection })
    }
}

impl BidiPageSession {
    pub(crate) fn connect(port: u16) -> Result<Self, PageError> {
        let connection = BidiConnection::connect(port)?;

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

impl PageSession for BidiPageSession {
    fn navigate(&mut self, url: &str) -> Result<(), PageError> {
        self.connection.navigate(url).map_err(PageError::Bidi)
    }

    fn evaluate_json(&mut self, expression: &str) -> Result<Value, PageError> {
        self.connection
            .evaluate_json(expression)
            .map_err(PageError::Bidi)
    }
}

impl PageSession for BrowserPageSession {
    fn navigate(&mut self, url: &str) -> Result<(), PageError> {
        match self {
            BrowserPageSession::Cdp(session) => session.navigate(url),
            BrowserPageSession::Bidi(session) => session.navigate(url),
        }
    }

    fn evaluate_json(&mut self, expression: &str) -> Result<Value, PageError> {
        match self {
            BrowserPageSession::Cdp(session) => session.evaluate_json(expression),
            BrowserPageSession::Bidi(session) => session.evaluate_json(expression),
        }
    }
}

#[derive(Debug)]
pub(crate) enum PageError {
    Base64(base64::DecodeError),
    Bidi(BidiError),
    Cdp(CdpError),
    Json(serde_json::Error),
    UnexpectedResponse(Value),
}

impl fmt::Display for PageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PageError::Base64(error) => write!(f, "{error}"),
            PageError::Bidi(error) => write!(f, "BiDi: {error}"),
            PageError::Cdp(error) => write!(f, "CDP: {error}"),
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

impl From<BidiError> for PageError {
    fn from(error: BidiError) -> Self {
        PageError::Bidi(error)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_error_names_cdp_protocol() {
        let error = PageError::Cdp(CdpError::ReadyStateTimeout);

        assert!(error.to_string().starts_with("CDP: "));
    }

    #[test]
    fn page_error_names_bidi_protocol() {
        let error = PageError::Bidi(BidiError::ConnectTimeout(9222));

        assert!(error.to_string().starts_with("BiDi: "));
    }
}
