use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::page::{PageError, PageSession};

pub(crate) const SCINET_URL: &str = "https://sci-net.xyz/";
const DOWNLOAD_CHUNK_SIZE: usize = 512 * 1024;

mod response;

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct SessionProbe {
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) text: String,
    pub(crate) token_balance: Option<u32>,
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
pub(crate) enum ScinetAvailability {
    OpenAccess,
    SciHub,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ScinetAvailabilityLink {
    pub(crate) source: ScinetAvailability,
    pub(crate) url: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RequestRemoteState {
    LoggedOut,
    NotFound,
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

    pub(crate) fn remote_state_for_doi(&self, doi: &str) -> RequestRemoteState {
        let state = self.remote_state();

        if matches!(state, RequestRemoteState::LoggedOut) || self.matches_doi(doi) {
            state
        } else {
            RequestRemoteState::NotFound
        }
    }

    pub(crate) fn matches_doi(&self, doi: &str) -> bool {
        let doi = doi.to_ascii_lowercase();

        if self.text.to_ascii_lowercase().contains(&doi) {
            return true;
        }

        self.url
            .eq_ignore_ascii_case(&request_url(SCINET_URL, &doi))
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
            || (text.contains("tokens") && text.contains("request"))
    }
}

pub(super) fn looks_like_login_text(text: &str) -> bool {
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
            RequestRemoteState::NotFound => "not-found",
            RequestRemoteState::Pdf => "pdf",
            RequestRemoteState::Working => "working",
            RequestRemoteState::Pending => "pending",
        }
    }
}

impl ScinetAvailability {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ScinetAvailability::OpenAccess => "open-access",
            ScinetAvailability::SciHub => "sci-hub",
        }
    }
}

pub(crate) fn probe_session(
    page: &mut impl PageSession,
    url: &str,
) -> Result<SessionProbe, PageError> {
    page.navigate(url)?;
    read_session(page)
}

pub(crate) fn probe_current_session(
    page: &mut impl PageSession,
) -> Result<SessionProbe, PageError> {
    read_session(page)
}

fn read_session(page: &mut impl PageSession) -> Result<SessionProbe, PageError> {
    let value = page.evaluate_json(
        r#"(async () => {
            const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

            const parseBalance = (value) => {
                if (typeof value === 'number' && Number.isFinite(value)) {
                    return Math.trunc(value);
                }

                if (typeof value !== 'string') {
                    return null;
                }

                const trimmed = value.replace(/,/g, '').trim();
                if (/^\d+$/.test(trimmed)) {
                    return Number(trimmed);
                }

                const tokenMatch = trimmed.match(/\btokens?\b\D+(\d+)/i)
                    || trimmed.match(/(\d+)\D+\btokens?\b/i);
                if (!tokenMatch) {
                    return null;
                }

                return Number(tokenMatch[1]);
            };

            const pageText = () => (document.body && document.body.innerText || '');
            const looksLoggedOut = () => {
                const text = pageText().toLowerCase();
                return text.includes('username')
                    && text.includes('password')
                    && (text.includes('no account yet') || text.includes('join'));
            };

            const readBalance = () => {
                const candidates = [];
                if (typeof window.balance !== 'undefined') {
                    candidates.push(window.balance);
                }

                for (const selector of [
                    'a.points',
                    '.controls .points',
                    '.points',
                    'a.points span',
                    '.controls .points span',
                    '.points span'
                ]) {
                    const element = document.querySelector(selector);
                    if (element) {
                        candidates.push(element.textContent || element.innerText || '');
                    }
                }

                for (const candidate of candidates) {
                    const parsed = parseBalance(candidate);
                    if (Number.isInteger(parsed) && parsed >= 0) {
                        return parsed;
                    }
                }

                return null;
            };

            let tokenBalance = readBalance();
            for (let attempt = 0; tokenBalance === null && attempt < 15 && !looksLoggedOut(); attempt += 1) {
                await sleep(100);
                tokenBalance = readBalance();
            }

            return {
                title: document.title,
                url: location.href,
                text: pageText().slice(0, 4000),
                token_balance: tokenBalance
            };
        })()"#,
    )?;

    serde_json::from_value(value).map_err(PageError::Json)
}

pub(crate) fn search_doi(
    page: &mut impl PageSession,
    url: &str,
    doi: &str,
) -> Result<ScinetResponse, PageError> {
    scinet_post(page, url, "/search", json!({ "doi": doi }))
}

pub(crate) fn request_doi(
    page: &mut impl PageSession,
    url: &str,
    doi: &str,
    reward: u32,
) -> Result<ScinetResponse, PageError> {
    scinet_post(
        page,
        url,
        "/request",
        json!({ "doi": doi, "reward": reward }),
    )
}

pub(crate) fn view_request(
    page: &mut impl PageSession,
    url: &str,
    doi: &str,
) -> Result<RequestView, PageError> {
    let view_url = request_url(url, doi);

    page.navigate(&view_url)?;

    let value = page.evaluate_json(
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

    serde_json::from_value(value).map_err(PageError::Json)
}

pub(crate) fn request_url(url: &str, doi: &str) -> String {
    let doi_path = doi_path(doi);
    format!("{}/{}", url.trim_end_matches('/'), doi_path)
}

pub(crate) fn download_pdf(
    page: &mut impl PageSession,
    pdf_url: &str,
) -> Result<PdfDownload, PageError> {
    let pdf_url = serde_json::to_string(pdf_url)?;

    let value = page.evaluate_json(&format!(
        r#"(async () => {{
            const response = await fetch({pdf_url}, {{ credentials: 'include' }});
            if (!response.ok) {{
                return {{
                    ok: false,
                    status: response.status,
                    content_type: response.headers.get('content-type'),
                    length: 0
                }};
            }}
            const bytes = new Uint8Array(await response.arrayBuffer());
            window.__snqDownloadBytes = bytes;
            return {{
                ok: true,
                status: response.status,
                content_type: response.headers.get('content-type'),
                length: bytes.length
            }};
        }})()"#
    ))?;

    let response: DownloadStartResponse = serde_json::from_value(value)?;

    if !response.ok {
        return Err(PageError::UnexpectedResponse(json!({
            "status": response.status,
            "content_type": response.content_type
        })));
    }

    let bytes = (|| {
        let mut bytes = Vec::with_capacity(response.length);

        for offset in (0..response.length).step_by(DOWNLOAD_CHUNK_SIZE) {
            let value = page.evaluate_json(&format!(
                r#"(() => {{
                    const bytes = window.__snqDownloadBytes;
                    if (!bytes) {{
                        throw new Error('missing snq download buffer');
                    }}
                    const slice = bytes.subarray({offset}, {offset} + {DOWNLOAD_CHUNK_SIZE});
                    let binary = '';
                    const chunk = 0x8000;
                    for (let i = 0; i < slice.length; i += chunk) {{
                        binary += String.fromCharCode(...slice.subarray(i, i + chunk));
                    }}
                    return btoa(binary);
                }})()"#
            ))?;
            let chunk: String = serde_json::from_value(value)?;
            bytes.extend(BASE64.decode(chunk)?);
        }

        Ok::<Vec<u8>, PageError>(bytes)
    })();

    let _ = page.evaluate_json("(() => { delete window.__snqDownloadBytes; return true; })()");
    let bytes = bytes?;

    Ok(PdfDownload {
        bytes,
        content_type: response.content_type,
    })
}

fn scinet_post(
    page: &mut impl PageSession,
    url: &str,
    path: &str,
    payload: Value,
) -> Result<ScinetResponse, PageError> {
    let path = serde_json::to_string(path)?;
    let payload = serde_json::to_string(&payload)?;

    page.navigate(url)?;

    let value = page.evaluate_json(&format!(
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

    serde_json::from_value(value).map_err(PageError::Json)
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
struct DownloadStartResponse {
    ok: bool,
    status: u16,
    content_type: Option<String>,
    length: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct FakePageSession {
        values: Vec<Value>,
        navigations: Vec<String>,
        expressions: Vec<String>,
    }

    impl FakePageSession {
        fn new(values: Vec<Value>) -> Self {
            Self {
                values,
                navigations: Vec::new(),
                expressions: Vec::new(),
            }
        }
    }

    impl PageSession for FakePageSession {
        fn navigate(&mut self, url: &str) -> Result<(), PageError> {
            self.navigations.push(url.to_string());
            Ok(())
        }

        fn evaluate_json(&mut self, expression: &str) -> Result<Value, PageError> {
            self.expressions.push(expression.to_string());

            if self.values.is_empty() {
                return Err(PageError::UnexpectedResponse(json!({
                    "error": "missing fake response"
                })));
            }

            Ok(self.values.remove(0))
        }

        fn close_browser(&mut self) -> Result<(), PageError> {
            Ok(())
        }
    }

    #[test]
    fn session_probe_uses_page_session_boundary() {
        let mut page = FakePageSession::new(vec![json!({
            "title": "Sci-Net",
            "url": "https://sci-net.xyz/",
            "text": "tokens request library"
        })]);

        let probe = probe_session(&mut page, SCINET_URL).unwrap();

        assert!(probe.is_logged_in());
        assert_eq!(probe.token_balance, None);
        assert_eq!(page.navigations, vec![SCINET_URL.to_string()]);
        assert_eq!(page.expressions.len(), 1);
        assert!(page.expressions[0].contains("document.body"));
    }

    #[test]
    fn session_probe_reports_token_balance() {
        let mut page = FakePageSession::new(vec![json!({
            "title": "Sci-Net",
            "url": "https://sci-net.xyz/",
            "text": "library tokens request",
            "token_balance": 8
        })]);

        let probe = probe_current_session(&mut page).unwrap();

        assert_eq!(probe.token_balance, Some(8));
        assert!(page.navigations.is_empty());
    }

    #[test]
    fn search_uses_page_session_boundary() {
        let mut page = FakePageSession::new(vec![json!({
            "ok": true,
            "status": 200,
            "body": { "ok": true }
        })]);

        let response = search_doi(&mut page, SCINET_URL, "10.1000/snq-example").unwrap();

        assert!(response.ok);
        assert_eq!(page.navigations, vec![SCINET_URL.to_string()]);
        assert!(page.expressions[0].contains("fetch(\"/search\""));
        assert!(page.expressions[0].contains("\"doi\":\"10.1000/snq-example\""));
    }

    #[test]
    fn detects_logged_in_session_text() {
        let probe = SessionProbe {
            title: "Sci-Net".to_string(),
            url: "https://sci-net.xyz/".to_string(),
            text: "tokens request library".to_string(),
            token_balance: Some(8),
        };

        assert!(probe.is_logged_in());
    }

    #[test]
    fn detects_logged_out_session_text() {
        let probe = SessionProbe {
            title: "Sci-Net".to_string(),
            url: "https://sci-net.xyz/".to_string(),
            text: "scientific communication support network No account yet? Join decentralized tokens reward request".to_string(),
            token_balance: None,
        };

        assert!(!probe.is_logged_in());
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
    fn request_view_matches_doi_from_body_text() {
        let view = RequestView {
            title: "Sci-Net".to_string(),
            url: "https://sci-net.xyz/request".to_string(),
            text: "Pending request for 10.1000/snq-example".to_string(),
            pdf_urls: Vec::new(),
        };

        assert!(view.matches_doi("10.1000/snq-example"));
    }

    #[test]
    fn request_view_matches_doi_from_request_url() {
        let doi = "10.1016/s0272-5231(21)01013-3";
        let view = RequestView {
            title: "Sci-Net".to_string(),
            url: request_url(SCINET_URL, doi),
            text: "Pending request".to_string(),
            pdf_urls: Vec::new(),
        };

        assert!(view.matches_doi(doi));
    }

    #[test]
    fn request_view_reports_not_found_for_unmatched_page() {
        let view = RequestView {
            title: "Sci-Net".to_string(),
            url: SCINET_URL.to_string(),
            text: "library tokens request active requests".to_string(),
            pdf_urls: Vec::new(),
        };

        assert_eq!(
            view.remote_state_for_doi("10.1000/snq-missing"),
            RequestRemoteState::NotFound
        );
    }

    #[test]
    fn request_view_keeps_matching_pending_state() {
        let view = RequestView {
            title: "Sci-Net: Pending".to_string(),
            url: "https://sci-net.xyz/request".to_string(),
            text: "Reward: 1 token 10.1000/snq-existing".to_string(),
            pdf_urls: Vec::new(),
        };

        assert_eq!(
            view.remote_state_for_doi("10.1000/snq-existing"),
            RequestRemoteState::Pending
        );
    }

    #[test]
    fn encodes_doi_as_one_path_segment() {
        assert_eq!(path_segment("10.1000/snq-example"), "10.1000%2Fsnq-example");
        assert_eq!(
            path_segment("10.1000/foo?bar#baz"),
            "10.1000%2Ffoo%3Fbar%23baz"
        );
    }

    #[test]
    fn encodes_doi_route_while_preserving_slash() {
        assert_eq!(doi_path("10.1000/snq-example"), "10.1000/snq-example");
        assert_eq!(doi_path("10.1000/foo?bar#baz"), "10.1000/foo%3Fbar%23baz");
    }

    #[test]
    fn builds_request_url_without_network_access() {
        assert_eq!(
            request_url("https://sci-net.xyz/", "10.1016/s0272-5231(21)01013-3"),
            "https://sci-net.xyz/10.1016/s0272-5231%2821%2901013-3"
        );
    }

    #[test]
    fn download_pdf_reassembles_chunks_and_cleans_browser_buffer() {
        let mut bytes = b"%PDF-1.7\n".to_vec();
        bytes.resize(DOWNLOAD_CHUNK_SIZE + 3, b'x');
        bytes[DOWNLOAD_CHUNK_SIZE..].copy_from_slice(b"end");

        let first_chunk = BASE64.encode(&bytes[..DOWNLOAD_CHUNK_SIZE]);
        let second_chunk = BASE64.encode(&bytes[DOWNLOAD_CHUNK_SIZE..]);
        let mut page = FakePageSession::new(vec![
            json!({
                "ok": true,
                "status": 200,
                "content_type": "application/pdf",
                "length": bytes.len()
            }),
            json!(first_chunk),
            json!(second_chunk),
            json!(true),
        ]);

        let download = download_pdf(&mut page, "https://sci-net.xyz/storage/paper.pdf").unwrap();

        assert_eq!(download.bytes, bytes);
        assert_eq!(download.content_type.as_deref(), Some("application/pdf"));
        assert_eq!(page.expressions.len(), 4);
        assert!(page.expressions[3].contains("delete window.__snqDownloadBytes"));
    }

    #[test]
    fn download_pdf_cleans_browser_buffer_after_chunk_error() {
        let mut page = FakePageSession::new(vec![
            json!({
                "ok": true,
                "status": 200,
                "content_type": "application/pdf",
                "length": 8
            }),
            json!(42),
            json!(true),
        ]);

        let error = download_pdf(&mut page, "https://sci-net.xyz/storage/paper.pdf").unwrap_err();

        assert!(error.to_string().contains("invalid type"));
        assert_eq!(page.expressions.len(), 3);
        assert!(page.expressions[2].contains("delete window.__snqDownloadBytes"));
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
                "https://sci-net.xyz/storage/fixture/example-fixture-paper.pdf#view=FitH&navpanes=0"
                    .to_string()
            ]
        );
        assert!(view.text.contains("Example Fixture Paper"));
    }

    fn fixture_view(name: &str) -> RequestView {
        let html = match name {
            "logged_out.html" => include_str!("../../tests/fixtures/scinet/logged_out.html"),
            "pending.html" => include_str!("../../tests/fixtures/scinet/pending.html"),
            "working.html" => include_str!("../../tests/fixtures/scinet/working.html"),
            "solved.html" => include_str!("../../tests/fixtures/scinet/solved.html"),
            _ => unreachable!("unknown fixture"),
        };

        RequestView {
            title: fixture_title(html),
            url: "https://sci-net.xyz/10.0000/snq-fixture".to_string(),
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
