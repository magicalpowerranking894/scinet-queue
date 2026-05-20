use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cdp::{CdpConnection, CdpError, page_target};

pub(crate) const SCINET_URL: &str = "https://sci-net.xyz/";

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct SessionProbe {
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) text: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ScinetResponse {
    pub(crate) ok: bool,
    pub(crate) status: u16,
    pub(crate) body: Value,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct RequestView {
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) text: String,
    pub(crate) pdf_urls: Vec<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RequestRemoteState {
    LoggedOut,
    Pdf,
    Working,
    Pending,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct PdfDownload {
    pub(crate) bytes: Vec<u8>,
    pub(crate) content_type: Option<String>,
}

impl RequestView {
    pub(crate) fn looks_logged_out(&self) -> bool {
        looks_like_login_text(&self.text)
    }

    pub(crate) fn has_pdf(&self) -> bool {
        !self.pdf_urls.is_empty()
    }

    pub(crate) fn remote_state(&self) -> RequestRemoteState {
        if self.looks_logged_out() {
            return RequestRemoteState::LoggedOut;
        }

        if self.has_pdf() {
            return RequestRemoteState::Pdf;
        }

        if looks_like_working_text(&self.text) {
            return RequestRemoteState::Working;
        }

        RequestRemoteState::Pending
    }
}

impl SessionProbe {
    pub(crate) fn is_logged_in(&self) -> bool {
        let text = self.text.to_ascii_lowercase();

        if text.contains("username")
            && text.contains("password")
            && (text.contains("no account yet") || text.contains("join"))
        {
            return false;
        }

        if text.contains("no account yet")
            || text.contains("scientific communication support network")
        {
            return false;
        }

        text.contains("logout")
            || text.contains("my requests")
            || text.contains("my library")
            || text.contains("user")
            || (text.contains("tokens") && text.contains("request"))
    }
}

impl ScinetResponse {
    pub(crate) fn looks_logged_out(&self) -> bool {
        self.body
            .get("text")
            .and_then(Value::as_str)
            .map(looks_like_login_text)
            .unwrap_or(false)
    }
}

fn looks_like_login_text(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("username") && text.contains("password") && text.contains("no account yet")
}

fn looks_like_working_text(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("working on solving")
        || text.contains("will upload pdf")
        || text.contains("is working on")
}

impl RequestRemoteState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            RequestRemoteState::LoggedOut => "logged-out",
            RequestRemoteState::Pdf => "pdf",
            RequestRemoteState::Working => "working",
            RequestRemoteState::Pending => "pending",
        }
    }
}

pub(crate) fn probe_session(port: u16, url: &str) -> Result<SessionProbe, CdpError> {
    let target = page_target(port)?;
    let mut cdp = CdpConnection::connect(&target.web_socket_debugger_url)?;

    cdp.navigate(url)?;
    read_session(&mut cdp)
}

pub(crate) fn probe_current_session(port: u16) -> Result<SessionProbe, CdpError> {
    let target = page_target(port)?;
    let mut cdp = CdpConnection::connect(&target.web_socket_debugger_url)?;

    read_session(&mut cdp)
}

fn read_session(cdp: &mut CdpConnection) -> Result<SessionProbe, CdpError> {
    let value = cdp.evaluate_json(
        "({ title: document.title, url: location.href, text: (document.body && document.body.innerText || '').slice(0, 4000) })",
    )?;

    serde_json::from_value(value).map_err(CdpError::Json)
}

pub(crate) fn search_doi(port: u16, url: &str, doi: &str) -> Result<ScinetResponse, CdpError> {
    scinet_post(port, url, "/search", json!({ "doi": doi }))
}

pub(crate) fn request_doi(
    port: u16,
    url: &str,
    doi: &str,
    reward: u32,
) -> Result<ScinetResponse, CdpError> {
    scinet_post(
        port,
        url,
        "/request",
        json!({ "doi": doi, "reward": reward }),
    )
}

pub(crate) fn view_request(port: u16, url: &str, doi: &str) -> Result<RequestView, CdpError> {
    let target = page_target(port)?;
    let mut cdp = CdpConnection::connect(&target.web_socket_debugger_url)?;
    let doi_path = doi_path(doi);
    let view_url = format!("{}/{}", url.trim_end_matches('/'), doi_path);

    cdp.navigate(&view_url)?;

    let value = cdp.evaluate_json(
        r#"(() => {
            const values = [];
            for (const element of document.querySelectorAll('a[href], iframe[src], embed[src], object[data]')) {
                const value = element.href || element.src || element.data;
                if (value && (value.includes('/storage/') || value.toLowerCase().includes('.pdf'))) {
                    values.push(value);
                }
            }
            return {
                title: document.title,
                url: location.href,
                text: (document.body && document.body.innerText || '').slice(0, 4000),
                pdf_urls: Array.from(new Set(values))
            };
        })()"#,
    )?;

    serde_json::from_value(value).map_err(CdpError::Json)
}

pub(crate) fn download_pdf(port: u16, pdf_url: &str) -> Result<PdfDownload, CdpError> {
    let target = page_target(port)?;
    let mut cdp = CdpConnection::connect(&target.web_socket_debugger_url)?;
    let pdf_url = serde_json::to_string(pdf_url)?;

    let value = cdp.evaluate_json(&format!(
        r#"(async () => {{
            const response = await fetch({pdf_url}, {{ credentials: 'include' }});
            const bytes = new Uint8Array(await response.arrayBuffer());
            let binary = '';
            const chunk = 0x8000;
            for (let i = 0; i < bytes.length; i += chunk) {{
                binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
            }}
            return {{
                ok: response.ok,
                status: response.status,
                content_type: response.headers.get('content-type'),
                body: btoa(binary)
            }};
        }})()"#
    ))?;

    let response: DownloadResponse = serde_json::from_value(value)?;

    if !response.ok {
        return Err(CdpError::UnexpectedResponse(json!({
            "status": response.status,
            "content_type": response.content_type
        })));
    }

    Ok(PdfDownload {
        bytes: BASE64.decode(response.body)?,
        content_type: response.content_type,
    })
}

fn scinet_post(
    port: u16,
    url: &str,
    path: &str,
    payload: Value,
) -> Result<ScinetResponse, CdpError> {
    let target = page_target(port)?;
    let mut cdp = CdpConnection::connect(&target.web_socket_debugger_url)?;
    let path = serde_json::to_string(path)?;
    let payload = serde_json::to_string(&payload)?;

    cdp.navigate(url)?;

    let value = cdp.evaluate_json(&format!(
        r#"(async () => {{
            const response = await fetch({path}, {{
                method: 'POST',
                credentials: 'include',
                headers: {{ 'content-type': 'application/json' }},
                body: JSON.stringify({payload})
            }});
            const text = await response.text();
            let body;
            try {{
                body = JSON.parse(text);
            }} catch (_) {{
                body = {{ text }};
            }}
            return {{ ok: response.ok, status: response.status, body }};
        }})()"#
    ))?;

    serde_json::from_value(value).map_err(CdpError::Json)
}

fn path_segment(value: &str) -> String {
    let mut encoded = String::new();

    for byte in value.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }

    encoded
}

fn doi_path(doi: &str) -> String {
    doi.split('/')
        .map(path_segment)
        .collect::<Vec<_>>()
        .join("/")
}

#[derive(Debug, Deserialize)]
struct DownloadResponse {
    ok: bool,
    status: u16,
    content_type: Option<String>,
    body: String,
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
            text: "scientific communication support network No account yet? Join decentralized tokens reward request".to_string(),
        };

        assert!(!probe.is_logged_in());
    }

    #[test]
    fn detects_logged_out_search_response() {
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: json!({
                "text": "<input name='user' placeholder='username'><input placeholder='password'>No account yet?"
            }),
        };

        assert!(response.looks_logged_out());
    }

    #[test]
    fn request_view_reports_pdf_presence() {
        let view = RequestView {
            title: "Sci-Net".to_string(),
            url: "https://sci-net.xyz/10.x".to_string(),
            text: "uploaded".to_string(),
            pdf_urls: vec!["https://sci-net.xyz/storage/paper.pdf".to_string()],
        };

        assert!(view.has_pdf());
        assert!(!view.looks_logged_out());
        assert_eq!(view.remote_state(), RequestRemoteState::Pdf);
    }

    #[test]
    fn request_view_reports_working_state() {
        let view = RequestView {
            title: "Sci-Net".to_string(),
            url: "https://sci-net.xyz/10.x".to_string(),
            text: "This member is working on solving the request and will upload PDF in 30 minutes"
                .to_string(),
            pdf_urls: Vec::new(),
        };

        assert_eq!(view.remote_state(), RequestRemoteState::Working);
    }

    #[test]
    fn encodes_doi_as_one_path_segment() {
        assert_eq!(
            path_segment("10.1287/mnsc.2024.05040"),
            "10.1287%2Fmnsc.2024.05040"
        );
        assert_eq!(
            path_segment("10.1000/foo?bar#baz"),
            "10.1000%2Ffoo%3Fbar%23baz"
        );
    }

    #[test]
    fn encodes_doi_route_while_preserving_slash() {
        assert_eq!(
            doi_path("10.1287/mnsc.2024.05040"),
            "10.1287/mnsc.2024.05040"
        );
        assert_eq!(doi_path("10.1000/foo?bar#baz"), "10.1000/foo%3Fbar%23baz");
    }

    #[test]
    fn fixture_detects_logged_out_state() {
        let view = fixture_view("logged_out.html");

        assert_eq!(view.remote_state(), RequestRemoteState::LoggedOut);
        assert!(view.pdf_urls.is_empty());
    }

    #[test]
    fn fixture_detects_pending_state() {
        let view = fixture_view("pending.html");

        assert_eq!(view.remote_state(), RequestRemoteState::Pending);
        assert!(view.pdf_urls.is_empty());
    }

    #[test]
    fn fixture_detects_working_state() {
        let view = fixture_view("working.html");

        assert_eq!(view.remote_state(), RequestRemoteState::Working);
        assert!(view.pdf_urls.is_empty());
    }

    #[test]
    fn fixture_detects_solved_pdf_state() {
        let view = fixture_view("solved.html");

        assert_eq!(view.remote_state(), RequestRemoteState::Pdf);
        assert_eq!(
            view.pdf_urls,
            vec![
                "https://sci-net.xyz/storage/6e3f106c16317628a337761c3aaaa768/Product-Variety-and-Asset-Pricing.pdf#view=FitH&navpanes=0"
                    .to_string()
            ]
        );
        assert!(view.text.contains("Product Variety and Asset Pricing"));
    }

    fn fixture_view(name: &str) -> RequestView {
        let html = match name {
            "logged_out.html" => include_str!("../tests/fixtures/scinet/logged_out.html"),
            "pending.html" => include_str!("../tests/fixtures/scinet/pending.html"),
            "working.html" => include_str!("../tests/fixtures/scinet/working.html"),
            "solved.html" => include_str!("../tests/fixtures/scinet/solved.html"),
            _ => unreachable!("unknown fixture"),
        };

        RequestView {
            title: fixture_title(html),
            url: "https://sci-net.xyz/10.1287/mnsc.2024.05040".to_string(),
            text: fixture_text(html),
            pdf_urls: fixture_pdf_urls(html),
        }
    }

    fn fixture_title(html: &str) -> String {
        between(html, "<title>", "</title>")
            .unwrap_or("Sci-Net")
            .trim()
            .to_string()
    }

    fn fixture_text(html: &str) -> String {
        let mut text = String::new();
        let mut in_tag = false;

        for ch in html.chars() {
            match ch {
                '<' => {
                    in_tag = true;
                    text.push(' ');
                }
                '>' => in_tag = false,
                _ if !in_tag => text.push(ch),
                _ => {}
            }
        }

        text.replace("&amp;", "&")
            .replace("&nbsp;", " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn fixture_pdf_urls(html: &str) -> Vec<String> {
        let mut urls = Vec::new();

        for attr in ["href=\"", "src=\"", "data=\""] {
            let mut rest = html;

            while let Some(index) = rest.find(attr) {
                rest = &rest[index + attr.len()..];
                let Some(end) = rest.find('"') else {
                    break;
                };
                let value = rest[..end].replace("&amp;", "&");

                if value.contains("/storage/") || value.to_ascii_lowercase().contains(".pdf") {
                    let url = if value.starts_with("http") {
                        value
                    } else {
                        format!("https://sci-net.xyz{value}")
                    };

                    if !urls.contains(&url) {
                        urls.push(url);
                    }
                }

                rest = &rest[end..];
            }
        }

        urls
    }

    fn between<'a>(value: &'a str, start: &str, end: &str) -> Option<&'a str> {
        let start_index = value.find(start)? + start.len();
        let rest = &value[start_index..];
        let end_index = rest.find(end)?;

        Some(&rest[..end_index])
    }
}
