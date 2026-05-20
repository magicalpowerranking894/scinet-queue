use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::page::{PageError, PageSession};

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
pub(crate) enum ScinetAvailability {
    OpenAccess,
    SciHub,
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

    pub(crate) fn logical_error(&self) -> Option<String> {
        if self.body.get("ok").and_then(Value::as_bool) == Some(false)
            || self.body.get("success").and_then(Value::as_bool) == Some(false)
        {
            return Some("response body reported failure".to_string());
        }

        for key in ["error", "errors"] {
            if let Some(value) = self.body.get(key).filter(|value| !value.is_null()) {
                return Some(format!("response body contained `{key}`: {value}"));
            }
        }

        let text = self
            .body
            .get("message")
            .or_else(|| self.body.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();

        if looks_like_login_text(&text)
            || text.contains("error")
            || text.contains("failed")
            || text.contains("invalid")
        {
            return Some("response body looked like an error".to_string());
        }

        None
    }

    pub(crate) fn availability(&self) -> Vec<ScinetAvailability> {
        if !self.ok || self.status >= 400 || self.logical_error().is_some() {
            return Vec::new();
        }

        let mut availability = Vec::new();
        collect_availability(None, &self.body, &mut availability);
        availability
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

impl ScinetAvailability {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ScinetAvailability::OpenAccess => "open-access",
            ScinetAvailability::SciHub => "sci-hub",
        }
    }
}

fn collect_availability(
    key: Option<&str>,
    value: &Value,
    availability: &mut Vec<ScinetAvailability>,
) {
    if let Some(kind) = key
        .and_then(availability_key)
        .filter(|_| value_is_present(value))
    {
        push_availability(availability, kind);
    }

    match value {
        Value::Object(map) => {
            if map.get("available").is_some_and(value_is_present) {
                collect_available_object_labels(map, availability);
            }

            for (key, value) in map {
                collect_availability(Some(key), value, availability);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_availability(None, value, availability);
            }
        }
        Value::Bool(true) => {
            if let Some(key) = key {
                collect_availability_text(key, availability);
            }
        }
        Value::String(text) => {
            collect_availability_text(text, availability);
        }
        _ => {}
    }
}

fn collect_available_object_labels(
    map: &serde_json::Map<String, Value>,
    availability: &mut Vec<ScinetAvailability>,
) {
    for (key, value) in map {
        if let Some(kind) = availability_key(key) {
            push_availability(availability, kind);
        }

        if let Value::String(text) = value {
            collect_provider_label(text, availability);
        }
    }
}

fn collect_provider_label(text: &str, availability: &mut Vec<ScinetAvailability>) {
    let text = text.to_ascii_lowercase();

    if text.contains("open access") || text.contains("open-access") || text.contains("openaccess") {
        push_availability(availability, ScinetAvailability::OpenAccess);
    }

    if text.contains("sci-hub") || text.contains("scihub") || text.contains("sci_hub") {
        push_availability(availability, ScinetAvailability::SciHub);
    }
}

fn availability_key(key: &str) -> Option<ScinetAvailability> {
    let key = key.to_ascii_lowercase();

    if key.contains("openaccess")
        || key.contains("open_access")
        || key.contains("open-access")
        || key.contains("open access")
    {
        return Some(ScinetAvailability::OpenAccess);
    }

    if key.contains("scihub") || key.contains("sci_hub") || key.contains("sci-hub") {
        return Some(ScinetAvailability::SciHub);
    }

    None
}

fn value_is_present(value: &Value) -> bool {
    match value {
        Value::Bool(value) => *value,
        Value::String(value) => {
            let value = value.trim().to_ascii_lowercase();
            !value.is_empty()
                && value != "false"
                && value != "none"
                && !value.contains("not found")
                && !value.contains("unavailable")
        }
        Value::Array(values) => values.iter().any(value_is_present),
        Value::Object(map) => map.values().any(value_is_present),
        _ => false,
    }
}

fn push_availability(availability: &mut Vec<ScinetAvailability>, kind: ScinetAvailability) {
    if !availability.contains(&kind) {
        availability.push(kind);
    }
}

fn collect_availability_text(text: &str, availability: &mut Vec<ScinetAvailability>) {
    let text = text.to_ascii_lowercase();

    if (text.contains("open access")
        || text.contains("open-access")
        || text.contains("openaccess")
        || text.contains("open_access"))
        && (text.contains("available")
            || text.contains("found")
            || text.contains("download")
            || text.contains("http"))
        && !text.contains("no open access")
        && !text.contains("open access not")
        && !text.contains("open-access not")
        && !text.contains("openaccess not")
        && !text.contains("open_access not")
        && !availability.contains(&ScinetAvailability::OpenAccess)
    {
        push_availability(availability, ScinetAvailability::OpenAccess);
    }

    if (text.contains("sci-hub") || text.contains("scihub") || text.contains("sci_hub"))
        && (text.contains("available")
            || text.contains("found")
            || text.contains("download")
            || text.contains("http"))
        && !text.contains("no sci-hub")
        && !text.contains("no scihub")
        && !text.contains("no sci_hub")
        && !text.contains("sci-hub not")
        && !text.contains("scihub not")
        && !text.contains("sci_hub not")
        && !availability.contains(&ScinetAvailability::SciHub)
    {
        push_availability(availability, ScinetAvailability::SciHub);
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
        "({ title: document.title, url: location.href, text: (document.body && document.body.innerText || '').slice(0, 4000) })",
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
    let doi_path = doi_path(doi);
    let view_url = format!("{}/{}", url.trim_end_matches('/'), doi_path);

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

pub(crate) fn download_pdf(
    page: &mut impl PageSession,
    pdf_url: &str,
) -> Result<PdfDownload, PageError> {
    let pdf_url = serde_json::to_string(pdf_url)?;

    let value = page.evaluate_json(&format!(
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
        return Err(PageError::UnexpectedResponse(json!({
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
struct DownloadResponse {
    ok: bool,
    status: u16,
    content_type: Option<String>,
    body: String,
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
        assert_eq!(page.navigations, vec![SCINET_URL.to_string()]);
        assert_eq!(page.expressions.len(), 1);
        assert!(page.expressions[0].contains("document.body"));
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
    fn detects_logical_error_response() {
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: json!({ "ok": false, "message": "invalid DOI" }),
        };

        assert!(response.logical_error().is_some());
    }

    #[test]
    fn detects_scinet_availability_from_search_response() {
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: json!({
                "open_access": true,
                "providers": [
                    { "name": "Sci-Hub", "available": true },
                    { "label": "publisher", "note": "Open Access version found" }
                ],
                "openaccess": "https://ojs.aaai.org/index.php/AAAI/article/download/33658/35813"
            }),
        };

        assert_eq!(
            response.availability(),
            vec![ScinetAvailability::OpenAccess, ScinetAvailability::SciHub]
        );
    }

    #[test]
    fn ignores_negative_availability_text() {
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: json!({
                "message": "No open access match. Sci-Hub not found.",
                "open_access": false,
                "sci_hub": false
            }),
        };

        assert!(response.availability().is_empty());
    }

    #[test]
    fn ignores_unavailable_provider_labels() {
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: json!({
                "providers": [
                    { "name": "Sci-Hub", "available": false },
                    { "name": "Open Access", "available": false }
                ]
            }),
        };

        assert!(response.availability().is_empty());
    }

    #[test]
    fn ignores_failed_availability_responses() {
        let response = ScinetResponse {
            ok: false,
            status: 500,
            body: json!({
                "openaccess": "https://example.test/paper.pdf",
                "sci_hub": true
            }),
        };

        assert!(response.availability().is_empty());
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
            "logged_out.html" => include_str!("../tests/fixtures/scinet/logged_out.html"),
            "pending.html" => include_str!("../tests/fixtures/scinet/pending.html"),
            "working.html" => include_str!("../tests/fixtures/scinet/working.html"),
            "solved.html" => include_str!("../tests/fixtures/scinet/solved.html"),
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
