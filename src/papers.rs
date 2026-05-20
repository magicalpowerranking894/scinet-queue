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
        let mut raw = tail
            .split(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\''))
            .next()
            .unwrap_or_default()
            .trim_end_matches(['.', ',', ';', ':', ')', ']', '}']);

        if is_doi_url_context(text, start) {
            raw = raw.split(['?', '#']).next().unwrap_or_default();
        }

        let raw = trim_adjacent_doi(raw);

        let Ok(doi) = normalize_doi(raw) else {
            continue;
        };

        if seen.insert(doi.clone()) {
            dois.push(doi);
        }
    }

    dois
}

fn is_doi_url_context(text: &str, start: usize) -> bool {
    let prefix = text[..start].to_ascii_lowercase();

    prefix.ends_with("https://doi.org/")
        || prefix.ends_with("http://doi.org/")
        || prefix.ends_with("https://dx.doi.org/")
        || prefix.ends_with("http://dx.doi.org/")
}

fn trim_adjacent_doi(raw: &str) -> &str {
    let bytes = raw.as_bytes();

    for (index, ch) in raw.char_indices() {
        if !matches!(ch, ',' | ';') {
            continue;
        }

        let mut next = index + ch.len_utf8();
        while next < bytes.len() && bytes[next].is_ascii_whitespace() {
            next += 1;
        }

        if raw[next..].starts_with("10.") {
            return &raw[..index];
        }
    }

    raw
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

    fs::create_dir_all(out_dir).map_err(|error| error.to_string())?;
    let out_path = output_path(out_dir, doi, pdf_url);
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
    let filename = pdf_filename(doi, pdf_url);
    let candidate = out_dir.join(&filename);

    if !candidate.exists() {
        return candidate;
    }

    let (stem, extension) = filename
        .rsplit_once('.')
        .map(|(stem, extension)| (stem.to_string(), format!(".{extension}")))
        .unwrap_or((filename, String::new()));

    for index in 2.. {
        let candidate = out_dir.join(format!("{stem}-{index}{extension}"));

        if !candidate.exists() {
            return candidate;
        }
    }

    unreachable!("unbounded filename suffix search cannot exhaust")
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
- query string https://doi.org/10.1000/ABC?utm_source=x
- angle wrapped <https://doi.org/10.1145/3544548.3581400>
"#;

        assert_eq!(
            extract_dois(text),
            vec![
                "10.1287/mnsc.2024.05040".to_string(),
                "10.1093/rfs/hhaa075".to_string(),
                "10.1000/abc".to_string(),
                "10.1145/3544548.3581400".to_string()
            ]
        );
    }

    #[test]
    fn extracts_old_style_dois_with_angle_brackets() {
        let text = "10.1002/(SICI)1097-4571(199505)46:4<327::AID-ASI5>3.0.CO;2-0";

        assert_eq!(
            extract_dois(text),
            vec!["10.1002/(sici)1097-4571(199505)46:4<327::aid-asi5>3.0.co;2-0"]
        );
    }

    #[test]
    fn extracts_adjacent_separator_delimited_dois() {
        assert_eq!(
            extract_dois("10.1000/one,10.1000/two;10.1000/three"),
            vec![
                "10.1000/one".to_string(),
                "10.1000/two".to_string(),
                "10.1000/three".to_string()
            ]
        );
    }

    #[test]
    fn import_does_not_truncate_literal_query_or_fragment_suffixes() {
        assert_eq!(
            extract_dois("literal query 10.5555/foo?bar"),
            Vec::<String>::new()
        );
        assert_eq!(
            extract_dois("literal fragment 10.6666/baz#frag"),
            Vec::<String>::new()
        );
        assert_eq!(
            extract_dois("url https://doi.org/10.1000/ABC?utm_source=x#frag"),
            vec!["10.1000/abc".to_string()]
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

    #[test]
    fn output_path_avoids_existing_files() {
        let dir = std::env::temp_dir().join(format!("snq-output-path-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("paper.pdf"), b"%PDF-1.7\n").unwrap();
        fs::write(dir.join("paper-2.pdf"), b"%PDF-1.7\n").unwrap();

        assert_eq!(
            output_path(&dir, "10.1000/one", "https://x/paper.pdf"),
            dir.join("paper-3.pdf")
        );

        let _ = fs::remove_dir_all(dir);
    }
}
