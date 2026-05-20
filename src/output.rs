use serde::Serialize;

use crate::queue::QueueStatus;
use crate::scinet::{RequestRemoteState, ScinetResponse};

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

A tiny agent-friendly DOI queue for Sci-Net.

Usage:
  snq login
  snq session [--json]
  snq add <doi>...
  snq import <path|->
  snq list [--json]
  snq remove <doi>
  snq check <doi>
  snq request <doi|--all> --reward <n> [--json]
  snq watch [--json]
  snq view <doi> [--json]
  snq fetch [<doi>] [--out <dir>] [--wait] [--poll <seconds>]
  snq approve <doi> [--force]
  snq doctor [--json]

Options:
      --json        Print machine-readable JSON where supported
      --no-wait     Open login browser without waiting for authentication
  -h, --help       Print help
  -V, --version    Print version
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
pub(crate) struct RequestOutput {
    pub(crate) doi: String,
    pub(crate) response: ScinetResponse,
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
}
