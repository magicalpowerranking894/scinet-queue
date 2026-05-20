mod browser;
mod cdp;
mod queue;

use std::env;
use std::process;

use std::fs;
use std::path::{Path, PathBuf};

use browser::{SCINET_URL, detect_browser, profile_dir};
use cdp::{
    RequestView, ScinetResponse, approve_doi, download_pdf, probe_session, request_doi, search_doi,
    view_request,
};
use queue::{
    AddResult, Queue, QueueStatus, RemoveResult, StatusResult, default_queue_path, normalize_doi,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    if let Err(error) = run() {
        eprintln!("snq: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let queue = Queue::new(default_queue_path());

    match args.next().as_deref() {
        None | Some("-h" | "--help" | "help") => {
            print_help();
        }
        Some("-V" | "--version") => {
            println!("snq {VERSION}");
        }
        Some("add") => {
            let Some(doi) = args.next() else {
                return Err("add: missing DOI".to_string());
            };

            match queue.add(&doi).map_err(|error| error.to_string())? {
                AddResult::Queued(doi) => println!("queued {doi}"),
                AddResult::AlreadyQueued(doi) => println!("already queued {doi}"),
            }
        }
        Some("list" | "ls") => {
            let entries = queue.list().map_err(|error| error.to_string())?;

            if entries.is_empty() {
                println!("queue empty");
            } else {
                for entry in entries {
                    println!("{}\t{}", entry.status, entry.doi);
                }
            }
        }
        Some("remove" | "rm") => {
            let Some(doi) = args.next() else {
                return Err("remove: missing DOI".to_string());
            };

            match queue.remove(&doi).map_err(|error| error.to_string())? {
                RemoveResult::Removed(doi) => println!("removed {doi}"),
                RemoveResult::NotFound(doi) => println!("not found {doi}"),
            }
        }
        Some("login") => {
            let browser = detect_browser().map_err(|error| error.to_string())?;
            let profile_dir = profile_dir(browser.engine).map_err(|error| error.to_string())?;
            let pid = browser
                .launch_login(&profile_dir)
                .map_err(|error| error.to_string())?;

            println!("opened {} browser pid {}", browser.engine, pid);
            println!("profile {}", profile_dir.display());
        }
        Some("session") => {
            let browser = detect_browser().map_err(|error| error.to_string())?;
            let profile_dir = profile_dir(browser.engine).map_err(|error| error.to_string())?;
            let cdp_browser = browser
                .launch_cdp(&profile_dir)
                .map_err(|error| error.to_string())?;
            let probe =
                probe_session(cdp_browser.port(), SCINET_URL).map_err(|error| error.to_string())?;

            println!("browser {}", browser.path.display());
            println!("engine {}", browser.engine);
            println!("profile {}", profile_dir.display());
            println!("queue {}", default_queue_path().display());
            println!("url {}", probe.url);
            println!("title {}", probe.title);
            println!(
                "login {}",
                if probe.is_logged_in() {
                    "detected"
                } else {
                    "not detected"
                }
            );
        }
        Some("check") => {
            let Some(doi) = args.next() else {
                return Err("check: missing DOI".to_string());
            };

            let doi = normalize_doi(&doi).map_err(|error| error.to_string())?;
            let response = with_scinet_session(|port| search_doi(port, SCINET_URL, &doi))?;
            let json = format_response(&response)?;

            println!("{json}");
        }
        Some("request") => {
            let Some(doi) = args.next() else {
                return Err("request: missing DOI".to_string());
            };

            let reward = parse_reward(args)?;
            let doi = normalize_doi(&doi).map_err(|error| error.to_string())?;
            let response = with_scinet_session(|port| request_doi(port, SCINET_URL, &doi, reward))?;
            let json = format_response(&response)?;

            match queue
                .set_status(&doi, QueueStatus::Requested)
                .map_err(|error| error.to_string())?
            {
                StatusResult::Updated(_) => {}
                StatusResult::NotFound(_) => {
                    let _ = queue.add(&doi).map_err(|error| error.to_string())?;
                    let _ = queue
                        .set_status(&doi, QueueStatus::Requested)
                        .map_err(|error| error.to_string())?;
                }
            }

            println!("{json}");
        }
        Some("watch") => {
            let entries = queue.list().map_err(|error| error.to_string())?;

            if entries.is_empty() {
                println!("queue empty");
                return Ok(());
            }

            let views = with_scinet_views(entries.iter().map(|entry| entry.doi.as_str()))?;

            for (entry, view) in entries.iter().zip(views.iter()) {
                let state = if view.has_pdf() {
                    "pdf"
                } else if view.looks_logged_out() {
                    "logged-out"
                } else {
                    "pending"
                };

                println!("{}\t{}\t{}", entry.status, state, entry.doi);
            }
        }
        Some("fetch") => {
            let fetch = parse_fetch(args)?;
            let dois = fetch_dois(&queue, fetch.doi.as_deref())?;

            if dois.is_empty() {
                println!("queue empty");
                return Ok(());
            }

            let outputs = with_scinet_port(|port| {
                let mut outputs = Vec::new();

                for doi in dois {
                    match fetch_one(&queue, port, &doi, &fetch.out_dir) {
                        Ok(Some(path)) => outputs.push(path),
                        Ok(None) => {}
                        Err(error) => return Err(error),
                    }
                }

                Ok(outputs)
            })?;

            if outputs.is_empty() {
                println!("no PDFs available");
            } else {
                for path in outputs {
                    println!("{}", path.display());
                }
            }
        }
        Some("approve") => {
            let Some(doi) = args.next() else {
                return Err("approve: missing DOI".to_string());
            };

            let doi = normalize_doi(&doi).map_err(|error| error.to_string())?;
            let response = with_scinet_session(|port| approve_doi(port, SCINET_URL, &doi))?;
            let json = format_response(&response)?;

            match queue
                .set_status(&doi, QueueStatus::Approved)
                .map_err(|error| error.to_string())?
            {
                StatusResult::Updated(_) => {}
                StatusResult::NotFound(_) => {}
            }

            println!("{json}");
        }
        Some(command) => {
            return Err(format!(
                "unknown command `{command}`\nrun `snq help` for usage"
            ));
        }
    }

    Ok(())
}

fn with_scinet_session<F>(operation: F) -> Result<ScinetResponse, String>
where
    F: FnOnce(u16) -> Result<ScinetResponse, cdp::CdpError>,
{
    with_scinet_port(|port| {
        let response = operation(port).map_err(|error| error.to_string())?;

        if response.looks_logged_out() {
            return Err("not logged into Sci-Net; run `snq login` first".to_string());
        }

        Ok(response)
    })
}

fn with_scinet_port<F, T>(operation: F) -> Result<T, String>
where
    F: FnOnce(u16) -> Result<T, String>,
{
    let browser = detect_browser().map_err(|error| error.to_string())?;
    let profile_dir = profile_dir(browser.engine).map_err(|error| error.to_string())?;
    let cdp_browser = browser
        .launch_cdp(&profile_dir)
        .map_err(|error| error.to_string())?;
    let probe = probe_session(cdp_browser.port(), SCINET_URL).map_err(|error| error.to_string())?;

    if !probe.is_logged_in() {
        return Err("not logged into Sci-Net; run `snq login` first".to_string());
    }

    operation(cdp_browser.port())
}

fn with_scinet_views<'a>(dois: impl Iterator<Item = &'a str>) -> Result<Vec<RequestView>, String> {
    with_scinet_port(|port| {
        let mut views = Vec::new();

        for doi in dois {
            views.push(view_request(port, SCINET_URL, doi).map_err(|error| error.to_string())?);
        }

        Ok(views)
    })
}

fn format_response(response: &ScinetResponse) -> Result<String, String> {
    serde_json::to_string_pretty(response).map_err(|error| error.to_string())
}

fn parse_reward(args: impl Iterator<Item = String>) -> Result<u32, String> {
    let mut reward = 1;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--reward" | "-r" => {
                let Some(value) = args.next() else {
                    return Err("request: missing value for --reward".to_string());
                };

                reward = value
                    .parse()
                    .map_err(|_| format!("request: invalid reward `{value}`"))?;

                if reward == 0 {
                    return Err("request: reward must be greater than zero".to_string());
                }
            }
            unknown => return Err(format!("request: unknown option `{unknown}`")),
        }
    }

    Ok(reward)
}

struct FetchArgs {
    doi: Option<String>,
    out_dir: PathBuf,
}

fn parse_fetch(args: impl Iterator<Item = String>) -> Result<FetchArgs, String> {
    let mut doi = None;
    let mut out_dir = std::path::PathBuf::from("papers");
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" | "-o" => {
                let Some(value) = args.next() else {
                    return Err("fetch: missing value for --out".to_string());
                };

                out_dir = value.into();
            }
            value if value.starts_with('-') => {
                return Err(format!("fetch: unknown option `{value}`"));
            }
            value => {
                if doi.is_some() {
                    return Err(format!("fetch: unexpected argument `{value}`"));
                }

                doi = Some(normalize_doi(value).map_err(|error| error.to_string())?);
            }
        }
    }

    Ok(FetchArgs { doi, out_dir })
}

fn fetch_dois(queue: &Queue, doi: Option<&str>) -> Result<Vec<String>, String> {
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

fn fetch_one(
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
        .split('?')
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

fn print_help() {
    println!(
        "\
snq {VERSION}

A tiny agent-friendly DOI queue for Sci-Net.

Usage:
  snq login
  snq session
  snq add <doi>
  snq list
  snq remove <doi>
  snq check <doi>
  snq request <doi> --reward <n>
  snq watch
  snq fetch [<doi>] [--out <dir>]
  snq approve <doi>

Options:
  -h, --help       Print help
  -V, --version    Print version
"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reward_defaults_to_one() {
        assert_eq!(parse_reward(std::iter::empty()).unwrap(), 1);
    }

    #[test]
    fn reward_accepts_long_and_short_flags() {
        assert_eq!(
            parse_reward(["--reward", "3"].into_iter().map(str::to_string)).unwrap(),
            3
        );
        assert_eq!(
            parse_reward(["-r", "2"].into_iter().map(str::to_string)).unwrap(),
            2
        );
    }

    #[test]
    fn reward_rejects_zero_missing_and_unknown_values() {
        assert!(parse_reward(["--reward", "0"].into_iter().map(str::to_string)).is_err());
        assert!(parse_reward(["--reward"].into_iter().map(str::to_string)).is_err());
        assert!(parse_reward(["--foo"].into_iter().map(str::to_string)).is_err());
    }

    #[test]
    fn out_dir_defaults_to_papers() {
        assert_eq!(
            parse_fetch(std::iter::empty()).unwrap().out_dir,
            std::path::PathBuf::from("papers")
        );
    }

    #[test]
    fn out_dir_accepts_long_and_short_flags() {
        assert_eq!(
            parse_fetch(["--out", "inbox"].into_iter().map(str::to_string))
                .unwrap()
                .out_dir,
            std::path::PathBuf::from("inbox")
        );
        assert_eq!(
            parse_fetch(["-o", "papers"].into_iter().map(str::to_string))
                .unwrap()
                .out_dir,
            std::path::PathBuf::from("papers")
        );
    }

    #[test]
    fn out_dir_rejects_missing_and_unknown_values() {
        assert!(parse_fetch(["--out"].into_iter().map(str::to_string)).is_err());
        assert!(parse_fetch(["--bad"].into_iter().map(str::to_string)).is_err());
    }

    #[test]
    fn fetch_accepts_optional_doi() {
        let args = parse_fetch(
            ["10.1287/mnsc.2024.05040", "--out", "papers"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();

        assert_eq!(args.doi.as_deref(), Some("10.1287/mnsc.2024.05040"));
        assert_eq!(args.out_dir, std::path::PathBuf::from("papers"));
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
    }

    #[test]
    fn pdf_filename_falls_back_to_doi() {
        assert_eq!(
            pdf_filename("10.1287/mnsc.2024.05040", "https://sci-net.xyz/view/x"),
            "10.1287-mnsc.2024.05040.pdf"
        );
    }
}
