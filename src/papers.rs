use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::queue::{Queue, QueueStatus, StatusResult, normalize_doi};
use crate::scinet::{SCINET_URL, download_pdf, view_request};

pub(crate) fn read_import_text(path: &str) -> Result<String, String> {
    if path == "-" {
        let mut input = String::new();
        io::stdin()
            .read_to_string(&mut input)
            .map_err(|error| error.to_string())?;
        Ok(input)
    } else {
        fs::read_to_string(path).map_err(|error| error.to_string())
    }
}

pub(crate) fn extract_dois(text: &str) -> Vec<String> {
    let mut dois = Vec::new();
    let mut seen = HashSet::new();

    for (start, _) in text.match_indices("10.") {
        let tail = &text[start..];
        let raw = tail
            .split(|ch: char| ch.is_whitespace() || matches!(ch, '<' | '>' | '"' | '\''))
            .next()
            .unwrap_or_default()
            .trim_end_matches(['.', ',', ';', ':', ')', ']', '}']);

        let Ok(doi) = normalize_doi(raw) else {
            continue;
        };

        if seen.insert(doi.clone()) {
            dois.push(doi);
        }
    }

    dois
}

pub(crate) fn fetch_dois(queue: &Queue, doi: Option<&str>) -> Result<Vec<String>, String> {
    if let Some(doi) = doi {
        return Ok(vec![normalize_doi(doi).map_err(|error| error.to_string())?]);
    }

    let entries = queue.list().map_err(|error| error.to_string())?;

    Ok(entries
        .into_iter()
        .filter(|entry| {
            matches!(
                entry.status,
                QueueStatus::Queued | QueueStatus::Requested | QueueStatus::Working
            )
        })
        .map(|entry| entry.doi)
        .collect())
}

pub(crate) fn fetch_one(
    queue: &Queue,
    port: u16,
    doi: &str,
    out_dir: &Path,
) -> Result<Option<PathBuf>, String> {
    let view = view_request(port, SCINET_URL, doi).map_err(|error| error.to_string())?;

    if view.looks_logged_out() {
        return Err("not logged into Sci-Net; run `snq login` first".to_string());
    }

    let Some(pdf_url) = view.pdf_urls.first() else {
        return Ok(None);
    };
    let download = download_pdf(port, pdf_url).map_err(|error| error.to_string())?;

    validate_pdf(&download.bytes)?;

    let out_path = output_path(out_dir, doi, pdf_url);

    fs::create_dir_all(out_dir).map_err(|error| error.to_string())?;
    fs::write(&out_path, download.bytes).map_err(|error| error.to_string())?;

    match queue
        .set_status(doi, QueueStatus::Fetched)
        .map_err(|error| error.to_string())?
    {
        StatusResult::Updated(_) => {}
        StatusResult::NotFound(_) => {}
    }

    Ok(Some(out_path))
}

fn validate_pdf(bytes: &[u8]) -> Result<(), String> {
    if bytes.starts_with(b"%PDF-") {
        Ok(())
    } else {
        Err("fetch: downloaded file is not a PDF".to_string())
    }
}

fn output_path(out_dir: &Path, doi: &str, pdf_url: &str) -> PathBuf {
    out_dir.join(pdf_filename(doi, pdf_url))
}

fn pdf_filename(doi: &str, pdf_url: &str) -> String {
    let tail = pdf_url
        .split(['?', '#'])
        .next()
        .and_then(|url| url.rsplit('/').next())
        .filter(|name| name.to_ascii_lowercase().ends_with(".pdf"))
        .filter(|name| !name.is_empty());

    tail.map(sanitize_filename)
        .unwrap_or_else(|| format!("{}.pdf", sanitize_filename(doi)))
}

fn sanitize_filename(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => ch,
            _ => '-',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_dois_from_markdown_text() {
        let text = r#"
- https://doi.org/10.1287/MNSC.2024.05040
- doi:10.1093/rfs/hhaa075.
- duplicate 10.1287/mnsc.2024.05040
"#;

        assert_eq!(
            extract_dois(text),
            vec![
                "10.1287/mnsc.2024.05040".to_string(),
                "10.1093/rfs/hhaa075".to_string()
            ]
        );
    }

    #[test]
    fn pdf_validation_rejects_non_pdf_bytes() {
        assert!(validate_pdf(b"%PDF-1.7\n").is_ok());
        assert!(validate_pdf(b"<html>").is_err());
    }

    #[test]
    fn pdf_filename_prefers_pdf_url_tail() {
        assert_eq!(
            pdf_filename(
                "10.1287/mnsc.2024.05040",
                "https://sci-net.xyz/storage/abc/Product Variety.pdf?token=x"
            ),
            "Product-Variety.pdf"
        );
        assert_eq!(
            pdf_filename(
                "10.1287/mnsc.2024.05040",
                "https://sci-net.xyz/storage/abc/Product Variety.pdf#view=FitH"
            ),
            "Product-Variety.pdf"
        );
    }

    #[test]
    fn pdf_filename_falls_back_to_doi() {
        assert_eq!(
            pdf_filename("10.1287/mnsc.2024.05040", "https://sci-net.xyz/view/x"),
            "10.1287-mnsc.2024.05040.pdf"
        );
    }
}
