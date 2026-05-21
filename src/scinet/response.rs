use serde_json::Value;

use super::{ScinetAvailability, ScinetResponse, looks_like_login_text};

const RESPONSE_REASON_KEYS: &[&str] = &[
    "message",
    "msg",
    "reason",
    "detail",
    "details",
    "description",
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

        let mut availability = Vec::new();
        collect_availability(None, &self.body, &mut availability);
        availability
    }
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
}
