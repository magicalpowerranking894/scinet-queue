use serde_json::Value;

use super::{ScinetAvailability, ScinetAvailabilityLink, ScinetResponse, looks_like_login_text};

const RESPONSE_REASON_KEYS: &[&str] = &[
    "message",
    "msg",
    "reason",
    "detail",
    "details",
    "description",
    "notFound",
    "not_found",
    "crossref",
    "text",
];

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
            if let Some(reason) = response_reason(&self.body) {
                return Some(format!("response body reported failure: {reason}"));
            }

            return Some("response body reported failure".to_string());
        }

        for key in ["error", "errors"] {
            if let Some(value) = self
                .body
                .get(key)
                .filter(|value| error_value_is_present(value))
            {
                if let Some(reason) = response_reason(&self.body) {
                    return Some(format!("response body reported `{key}`: {reason}"));
                }

                if value.as_bool() == Some(true) {
                    return Some(format!(
                        "response body reported `{key}`=true without a reason"
                    ));
                }

                return Some(format!("response body contained `{key}`: {value}"));
            }
        }

        let raw_text = self
            .body
            .get("message")
            .or_else(|| self.body.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let text = raw_text.to_ascii_lowercase();

        if looks_like_login_text(&text)
            || text.contains("error")
            || text.contains("failed")
            || text.contains("invalid")
            || text.contains("not found")
            || text.contains("crossref")
        {
            if let Some(reason) = compact_reason(raw_text) {
                return Some(format!("response body looked like an error: {reason}"));
            }

            return Some("response body looked like an error".to_string());
        }

        None
    }

    pub(crate) fn availability(&self) -> Vec<ScinetAvailability> {
        if !self.ok || self.status >= 400 || self.logical_error().is_some() {
            return Vec::new();
        }

        let mut info = AvailabilityInfo::default();
        collect_availability(None, &self.body, &mut info);
        info.kinds
    }

    pub(crate) fn availability_links(&self) -> Vec<ScinetAvailabilityLink> {
        if !self.ok || self.status >= 400 || self.logical_error().is_some() {
            return Vec::new();
        }

        let mut info = AvailabilityInfo::default();
        collect_availability(None, &self.body, &mut info);
        info.links
    }
}

#[derive(Default)]
struct AvailabilityInfo {
    kinds: Vec<ScinetAvailability>,
    links: Vec<ScinetAvailabilityLink>,
}

fn response_reason(body: &Value) -> Option<String> {
    let map = body.as_object()?;

    for key in RESPONSE_REASON_KEYS {
        if let Some(reason) = map.get(*key).and_then(reason_from_value) {
            return Some(reason);
        }
    }

    for key in ["error", "errors"] {
        if let Some(reason) = map.get(key).and_then(reason_from_value) {
            return Some(reason);
        }
    }

    None
}

fn reason_from_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => compact_reason(text),
        Value::Array(values) => values
            .iter()
            .find_map(reason_from_value)
            .or_else(|| compact_json_reason(value)),
        Value::Object(map) => RESPONSE_REASON_KEYS
            .iter()
            .find_map(|key| map.get(*key).and_then(reason_from_value))
            .or_else(|| compact_json_reason(value)),
        Value::Number(_) => Some(value.to_string()),
        _ => None,
    }
}

fn compact_json_reason(value: &Value) -> Option<String> {
    if matches!(value, Value::Null | Value::Bool(_)) {
        return None;
    }

    compact_reason(&value.to_string())
}

fn compact_reason(text: &str) -> Option<String> {
    const MAX_REASON_CHARS: usize = 240;

    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");

    if compact.is_empty() {
        return None;
    }

    if compact.chars().count() <= MAX_REASON_CHARS {
        return Some(compact);
    }

    let mut truncated = compact
        .chars()
        .take(MAX_REASON_CHARS.saturating_sub(3))
        .collect::<String>();
    truncated.push_str("...");
    Some(truncated)
}

fn error_value_is_present(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::String(value) => {
            let value = value.trim();
            !value.is_empty() && !value.eq_ignore_ascii_case("false")
        }
        Value::Array(values) => !values.is_empty(),
        Value::Object(map) => !map.is_empty(),
        _ => true,
    }
}

fn collect_availability(key: Option<&str>, value: &Value, info: &mut AvailabilityInfo) {
    if let Some(kind) = key.and_then(availability_key) {
        if value_is_present(value) {
            push_availability(info, kind);
        }

        for url in value_urls(value) {
            push_availability_link(info, kind, url);
        }
    }

    match value {
        Value::Object(map) => {
            if map.get("available").is_some_and(value_is_present) {
                collect_available_object_labels(map, info);
            }

            for (key, value) in map {
                collect_availability(Some(key), value, info);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_availability(None, value, info);
            }
        }
        Value::Bool(true) => {
            if let Some(key) = key {
                collect_availability_text(key, info);
            }
        }
        Value::String(text) => {
            collect_availability_text(text, info);
        }
        _ => {}
    }
}

fn collect_available_object_labels(
    map: &serde_json::Map<String, Value>,
    info: &mut AvailabilityInfo,
) {
    let mut kinds = Vec::new();

    for (key, value) in map {
        if let Some(kind) = availability_key(key) {
            push_kind(&mut kinds, kind);
        }

        if let Value::String(text) = value {
            collect_provider_label(text, &mut kinds);
        }
    }

    for kind in &kinds {
        push_availability(info, *kind);
    }

    for kind in kinds {
        for url in object_urls(map) {
            push_availability_link(info, kind, url);
        }
    }
}

fn collect_provider_label(text: &str, kinds: &mut Vec<ScinetAvailability>) {
    let text = text.to_ascii_lowercase();

    if text.contains("open access") || text.contains("open-access") || text.contains("openaccess") {
        push_kind(kinds, ScinetAvailability::OpenAccess);
    }

    if text.contains("sci-hub") || text.contains("scihub") || text.contains("sci_hub") {
        push_kind(kinds, ScinetAvailability::SciHub);
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

fn push_availability(info: &mut AvailabilityInfo, kind: ScinetAvailability) {
    push_kind(&mut info.kinds, kind);
}

fn push_kind(kinds: &mut Vec<ScinetAvailability>, kind: ScinetAvailability) {
    if !kinds.contains(&kind) {
        kinds.push(kind);
    }
}

fn push_availability_link(info: &mut AvailabilityInfo, source: ScinetAvailability, url: &str) {
    push_availability(info, source);

    let url = clean_url(url);

    if url.is_empty() {
        return;
    }

    if !info
        .links
        .iter()
        .any(|link| link.source == source && link.url == url)
    {
        info.links.push(ScinetAvailabilityLink { source, url });
    }
}

fn collect_availability_text(text: &str, info: &mut AvailabilityInfo) {
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
    {
        push_availability(info, ScinetAvailability::OpenAccess);
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
    {
        push_availability(info, ScinetAvailability::SciHub);
    }
}

fn object_urls(map: &serde_json::Map<String, Value>) -> Vec<&str> {
    map.values().flat_map(value_urls).collect()
}

fn value_urls(value: &Value) -> Vec<&str> {
    let mut urls = Vec::new();
    collect_value_urls(value, &mut urls);
    urls
}

fn collect_value_urls<'a>(value: &'a Value, urls: &mut Vec<&'a str>) {
    match value {
        Value::String(text) => {
            if is_url(text) {
                urls.push(text);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_value_urls(value, urls);
            }
        }
        Value::Object(map) => {
            for value in map.values() {
                collect_value_urls(value, urls);
            }
        }
        _ => {}
    }
}

fn is_url(value: &str) -> bool {
    let value = value.trim();
    value.starts_with("https://") || value.starts_with("http://") || value.starts_with("//")
}

fn clean_url(value: &str) -> String {
    let value = value
        .trim()
        .trim_matches(|ch| matches!(ch, '"' | '\'' | '<' | '>' | ')' | ']' | '}' | ',' | ';'))
        .to_string();

    if value.starts_with("//") {
        format!("https:{value}")
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

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

        let error = response.logical_error().unwrap();
        assert!(error.contains("invalid DOI"));
    }

    #[test]
    fn reports_scinet_error_reason() {
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: json!({ "error": true, "message": "not enough tokens" }),
        };

        let error = response.logical_error().unwrap();
        assert!(error.contains("not enough tokens"));
    }

    #[test]
    fn reports_boolean_scinet_error_without_reason() {
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: json!({ "error": true }),
        };

        let error = response.logical_error().unwrap();
        assert!(error.contains("error`=true"));
    }

    #[test]
    fn reports_crossref_not_found_error_reason() {
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: json!({
                "error": true,
                "crossref": "DOI was not found in Crossref"
            }),
        };

        let error = response.logical_error().unwrap();
        assert!(error.contains("DOI was not found in Crossref"));
    }

    #[test]
    fn reports_plain_text_not_found_error() {
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: json!({
                "text": "DOI not found by Crossref. Check the DOI and try again."
            }),
        };

        let error = response.logical_error().unwrap();
        assert!(error.contains("DOI not found by Crossref"));
    }

    #[test]
    fn ignores_false_scinet_error_field() {
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: json!({ "error": false }),
        };

        assert!(response.logical_error().is_none());
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
        assert_eq!(
            response.availability_links(),
            vec![ScinetAvailabilityLink {
                source: ScinetAvailability::OpenAccess,
                url: "https://ojs.aaai.org/index.php/AAAI/article/download/33658/35813".to_string(),
            }]
        );
    }

    #[test]
    fn detects_availability_links_from_provider_objects() {
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: json!({
                "providers": [
                    {
                        "name": "Sci-Hub",
                        "available": true,
                        "url": "https://sci-hub.example/10.1000/snq-example"
                    },
                    {
                        "label": "Open Access",
                        "available": true,
                        "href": "https://open.example/paper.pdf"
                    }
                ]
            }),
        };

        assert_eq!(
            response.availability_links(),
            vec![
                ScinetAvailabilityLink {
                    source: ScinetAvailability::SciHub,
                    url: "https://sci-hub.example/10.1000/snq-example".to_string(),
                },
                ScinetAvailabilityLink {
                    source: ScinetAvailability::OpenAccess,
                    url: "https://open.example/paper.pdf".to_string(),
                }
            ]
        );
    }

    #[test]
    fn normalizes_protocol_relative_availability_links() {
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: json!({
                "sci-hub": "//sci-hub.example/10.1000/snq-example"
            }),
        };

        assert_eq!(
            response.availability_links(),
            vec![ScinetAvailabilityLink {
                source: ScinetAvailability::SciHub,
                url: "https://sci-hub.example/10.1000/snq-example".to_string(),
            }]
        );
    }

    #[test]
    fn ignores_unrelated_urls_in_availability_links() {
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: json!({
                "publisher": "https://publisher.example/paper",
                "open_access": false,
                "providers": [
                    {
                        "name": "publisher",
                        "available": true,
                        "url": "https://publisher.example/landing"
                    }
                ]
            }),
        };

        assert!(response.availability_links().is_empty());
    }

    #[test]
    fn deduplicates_availability_links() {
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: json!({
                "sci_hub": "https://sci-hub.example/10.1000/snq-example",
                "providers": [
                    {
                        "name": "Sci-Hub",
                        "available": true,
                        "url": "https://sci-hub.example/10.1000/snq-example"
                    }
                ]
            }),
        };

        assert_eq!(
            response.availability_links(),
            vec![ScinetAvailabilityLink {
                source: ScinetAvailability::SciHub,
                url: "https://sci-hub.example/10.1000/snq-example".to_string(),
            }]
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
        assert!(response.availability_links().is_empty());
    }
}
