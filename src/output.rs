use serde::Serialize;

use crate::queue::QueueStatus;
use crate::scinet::{
    RequestRemoteState, ScinetAvailability, ScinetAvailabilityLink, ScinetResponse,
};

pub(crate) fn format_response(response: &ScinetResponse) -> Result<String, String> {
    serde_json::to_string_pretty(response).map_err(|error| error.to_string())
}

pub(crate) fn print_json<T: Serialize>(value: &T) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|error| error.to_string())?
    );
    Ok(())
}

pub(crate) fn compact_text(text: &str) -> String {
    text.split_whitespace()
        .take(120)
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn print_help(version: &str) {
    println!(
        "\
snq {version}

A tiny scriptable queue for Sci-Net paper requests.

Usage:
  snq login [--no-wait]
  snq session [--json]
  snq browsers [--pick|--set <path>|--clear] [--json]
  snq add <doi>...
  snq import <path|->
  snq list [--json]
  snq remove <doi>
  snq check <doi>
  snq request <doi|--all> [--reward <n>] [--json]
  snq watch [--json]
  snq view <doi> [--json]
  snq url <doi>
  snq fetch [<doi>] [--out <dir>] [--wait] [--poll <seconds>] [--json]
  snq approve <doi> [--force] [--json]
  snq doctor [--json]

Options:
      --json        Print machine-readable JSON where supported
  -h, --help       Print help
  -V, --version    Print version

Command-specific options:
      --no-wait     Open login browser without waiting for authentication
      --wait        Poll fetch targets until each has a PDF or availability hint
"
    );
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionOutput {
    pub(crate) browser: String,
    pub(crate) engine: String,
    pub(crate) profile: String,
    pub(crate) queue: String,
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) logged_in: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct BrowserListOutput {
    pub(crate) override_env: String,
    pub(crate) preference_path: String,
    pub(crate) preference_error: Option<String>,
    pub(crate) selected: Option<BrowserChoiceOutput>,
    pub(crate) browsers: Vec<BrowserChoiceOutput>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct BrowserChoiceOutput {
    pub(crate) selected: bool,
    pub(crate) available: bool,
    pub(crate) engine: String,
    pub(crate) source: String,
    pub(crate) path: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct RequestOutput {
    pub(crate) doi: String,
    pub(crate) status: QueueStatus,
    pub(crate) remote_state: Option<RequestRemoteState>,
    pub(crate) response: ScinetResponse,
}

#[derive(Debug, Serialize)]
pub(crate) struct FetchOutput {
    pub(crate) doi: String,
    pub(crate) status: FetchOutputStatus,
    pub(crate) remote_state: RequestRemoteState,
    pub(crate) availability: Vec<ScinetAvailability>,
    pub(crate) availability_links: Vec<ScinetAvailabilityLink>,
    pub(crate) path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FetchOutputStatus {
    Fetched,
    NoPdf,
}

#[derive(Debug, Serialize)]
pub(crate) struct ApproveOutput {
    pub(crate) doi: String,
    pub(crate) status: QueueStatus,
    pub(crate) forced: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct WatchOutput {
    pub(crate) doi: String,
    pub(crate) status: QueueStatus,
    pub(crate) remote_state: RequestRemoteState,
}

#[derive(Debug, Serialize)]
pub(crate) struct ViewOutput {
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) state: RequestRemoteState,
    pub(crate) pdf_urls: Vec<String>,
    pub(crate) text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_text_collapses_whitespace_and_truncates() {
        let text = (0..130)
            .map(|index| format!("word{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let compact = compact_text(&text);

        assert_eq!(compact.split_whitespace().count(), 120);
        assert!(compact.starts_with("word0 word1"));
        assert!(!compact.contains("word129"));
    }

    #[test]
    fn fetch_output_includes_remote_state() {
        let output = FetchOutput {
            doi: "10.1000/snq-example".to_string(),
            status: FetchOutputStatus::NoPdf,
            remote_state: RequestRemoteState::Working,
            availability: vec![ScinetAvailability::SciHub],
            availability_links: vec![ScinetAvailabilityLink {
                source: ScinetAvailability::SciHub,
                url: "https://sci-hub.example/10.1000/snq-example".to_string(),
            }],
            path: None,
        };
        let value = serde_json::to_value(output).unwrap();

        assert_eq!(value["status"], "no-pdf");
        assert_eq!(value["remote_state"], "working");
        assert_eq!(value["availability"], serde_json::json!(["sci-hub"]));
        assert_eq!(
            value["availability_links"],
            serde_json::json!([
                {
                    "source": "sci-hub",
                    "url": "https://sci-hub.example/10.1000/snq-example"
                }
            ])
        );
        assert!(value["path"].is_null());
    }
}
