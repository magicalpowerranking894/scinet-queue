mod browser;
mod cdp;
mod queue;

use std::collections::HashSet;
use std::env;
use std::process;

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use browser::{SCINET_URL, detect_browser, profile_dir};
use cdp::{
    RequestRemoteState, RequestView, ScinetResponse, approve_doi, download_pdf,
    probe_current_session, probe_session, request_doi, search_doi, view_request,
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
            let dois = args.collect::<Vec<_>>();

            if dois.is_empty() {
                return Err("add: missing DOI".to_string());
            }

            for doi in dois {
                match queue.add(&doi).map_err(|error| error.to_string())? {
                    AddResult::Queued(doi) => println!("queued {doi}"),
                    AddResult::AlreadyQueued(doi) => println!("already queued {doi}"),
                }
            }
        }
        Some("import") => {
            let Some(path) = args.next() else {
                return Err("import: missing path".to_string());
            };

            let text = read_import_text(&path)?;
            let dois = extract_dois(&text);

            if dois.is_empty() {
                println!("no DOIs found");
                return Ok(());
            }

            for doi in dois {
                match queue.add(&doi).map_err(|error| error.to_string())? {
                    AddResult::Queued(doi) => println!("queued {doi}"),
                    AddResult::AlreadyQueued(doi) => println!("already queued {doi}"),
                }
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
            let login = parse_login(args)?;
            let browser = detect_browser().map_err(|error| error.to_string())?;
            let profile_dir = profile_dir(browser.engine).map_err(|error| error.to_string())?;

            if login.wait {
                let cdp_browser = browser
                    .launch_login_cdp(&profile_dir)
                    .map_err(|error| error.to_string())?;

                println!("opened {}", browser.engine);
                println!("profile {}", profile_dir.display());
                println!("waiting for Sci-Net login; press Ctrl-C to cancel");

                wait_for_login(cdp_browser.port())?;
                println!("login detected");
            } else {
                let pid = browser
                    .launch_login(&profile_dir)
                    .map_err(|error| error.to_string())?;

                println!("opened {} browser pid {}", browser.engine, pid);
                println!("profile {}", profile_dir.display());
            }
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
            let request = parse_request(args)?;
            let dois = request_dois(&queue, &request)?;

            if dois.is_empty() {
                println!("no queued entries");
                return Ok(());
            }

            let responses = with_scinet_port(|port| {
                let mut responses = Vec::new();

                for doi in &dois {
                    let response = request_doi(port, SCINET_URL, doi, request.reward)
                        .map_err(|error| error.to_string())?;

                    if response.looks_logged_out() {
                        return Err("not logged into Sci-Net; run `snq login` first".to_string());
                    }

                    mark_requested(&queue, doi)?;
                    responses.push(response);
                }

                Ok(responses)
            })?;

            for (doi, response) in dois.iter().zip(responses.iter()) {
                if request.all {
                    println!("requested\t{doi}");
                } else {
                    println!("{}", format_response(response)?);
                }
            }
        }
        Some("watch") => {
            let entries = queue.list().map_err(|error| error.to_string())?;

            if entries.is_empty() {
                println!("queue empty");
                return Ok(());
            }

            let views = with_scinet_views(entries.iter().map(|entry| entry.doi.as_str()))?;

            for (entry, view) in entries.iter().zip(views.iter()) {
                let remote_state = view.remote_state();
                let status =
                    update_status_from_remote(&queue, entry.status, &entry.doi, remote_state)?;

                println!("{}\t{}\t{}", status, remote_state.as_str(), entry.doi);
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
                loop {
                    let mut outputs = Vec::new();

                    for doi in &dois {
                        match fetch_one(&queue, port, doi, &fetch.out_dir) {
                            Ok(Some(path)) => outputs.push(path),
                            Ok(None) => {}
                            Err(error) => return Err(error),
                        }
                    }

                    if !outputs.is_empty() || !fetch.wait {
                        return Ok(outputs);
                    }

                    println!("no PDFs available; waiting {}s", fetch.poll_secs);
                    thread::sleep(Duration::from_secs(fetch.poll_secs));
                }
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
            let approve = parse_approve(args)?;
            let doi = approve.doi;
            ensure_can_approve(&queue, &doi, approve.force)?;
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

struct LoginArgs {
    wait: bool,
}

fn parse_login(args: impl Iterator<Item = String>) -> Result<LoginArgs, String> {
    let mut wait = true;

    for arg in args {
        match arg.as_str() {
            "--no-wait" => wait = false,
            unknown => return Err(format!("login: unknown option `{unknown}`")),
        }
    }

    Ok(LoginArgs { wait })
}

fn wait_for_login(port: u16) -> Result<(), String> {
    loop {
        let probe = probe_current_session(port).map_err(|error| error.to_string())?;

        if probe.is_logged_in() {
            return Ok(());
        }

        thread::sleep(Duration::from_secs(1));
    }
}

fn update_status_from_remote(
    queue: &Queue,
    status: QueueStatus,
    doi: &str,
    remote_state: RequestRemoteState,
) -> Result<QueueStatus, String> {
    if remote_state == RequestRemoteState::Working
        && matches!(status, QueueStatus::Queued | QueueStatus::Requested)
    {
        queue
            .set_status(doi, QueueStatus::Working)
            .map_err(|error| error.to_string())?;

        Ok(QueueStatus::Working)
    } else {
        Ok(status)
    }
}

fn read_import_text(path: &str) -> Result<String, String> {
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

fn extract_dois(text: &str) -> Vec<String> {
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

struct RequestArgs {
    doi: Option<String>,
    reward: u32,
    all: bool,
}

fn parse_request(args: impl Iterator<Item = String>) -> Result<RequestArgs, String> {
    let mut doi = None;
    let mut reward = 1;
    let mut all = false;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--all" => all = true,
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
            value if value.starts_with('-') => {
                return Err(format!("request: unknown option `{value}`"));
            }
            value => {
                if doi.is_some() {
                    return Err(format!("request: unexpected argument `{value}`"));
                }

                doi = Some(normalize_doi(value).map_err(|error| error.to_string())?);
            }
        }
    }

    if all && doi.is_some() {
        return Err("request: use either --all or one DOI".to_string());
    }

    if !all && doi.is_none() {
        return Err("request: missing DOI".to_string());
    }

    Ok(RequestArgs { doi, reward, all })
}

fn request_dois(queue: &Queue, request: &RequestArgs) -> Result<Vec<String>, String> {
    if let Some(doi) = &request.doi {
        return Ok(vec![doi.clone()]);
    }

    let entries = queue.list().map_err(|error| error.to_string())?;

    Ok(entries
        .into_iter()
        .filter(|entry| entry.status == QueueStatus::Queued)
        .map(|entry| entry.doi)
        .collect())
}

fn mark_requested(queue: &Queue, doi: &str) -> Result<(), String> {
    match queue
        .set_status(doi, QueueStatus::Requested)
        .map_err(|error| error.to_string())?
    {
        StatusResult::Updated(_) => {}
        StatusResult::NotFound(_) => {
            let _ = queue.add(doi).map_err(|error| error.to_string())?;
            let _ = queue
                .set_status(doi, QueueStatus::Requested)
                .map_err(|error| error.to_string())?;
        }
    }

    Ok(())
}

struct FetchArgs {
    doi: Option<String>,
    out_dir: PathBuf,
    wait: bool,
    poll_secs: u64,
}

struct ApproveArgs {
    doi: String,
    force: bool,
}

fn parse_approve(args: impl Iterator<Item = String>) -> Result<ApproveArgs, String> {
    let mut doi = None;
    let mut force = false;

    for arg in args {
        match arg.as_str() {
            "--force" => force = true,
            value if value.starts_with('-') => {
                return Err(format!("approve: unknown option `{value}`"));
            }
            value => {
                if doi.is_some() {
                    return Err(format!("approve: unexpected argument `{value}`"));
                }

                doi = Some(normalize_doi(value).map_err(|error| error.to_string())?);
            }
        }
    }

    let Some(doi) = doi else {
        return Err("approve: missing DOI".to_string());
    };

    Ok(ApproveArgs { doi, force })
}

fn ensure_can_approve(queue: &Queue, doi: &str, force: bool) -> Result<(), String> {
    if force {
        return Ok(());
    }

    let entries = queue.list().map_err(|error| error.to_string())?;

    if entries
        .iter()
        .any(|entry| entry.doi == doi && entry.status == QueueStatus::Fetched)
    {
        Ok(())
    } else {
        Err(format!(
            "approve: {doi} is not fetched; run `snq fetch {doi}` first or pass --force"
        ))
    }
}

fn parse_fetch(args: impl Iterator<Item = String>) -> Result<FetchArgs, String> {
    let mut doi = None;
    let mut out_dir = std::path::PathBuf::from("papers");
    let mut wait = false;
    let mut poll_secs = 30;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--wait" => wait = true,
            "--poll" => {
                let Some(value) = args.next() else {
                    return Err("fetch: missing value for --poll".to_string());
                };

                poll_secs = value
                    .parse()
                    .map_err(|_| format!("fetch: invalid poll interval `{value}`"))?;

                if poll_secs == 0 {
                    return Err("fetch: poll interval must be greater than zero".to_string());
                }
            }
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

    Ok(FetchArgs {
        doi,
        out_dir,
        wait,
        poll_secs,
    })
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
  snq add <doi>...
  snq import <path|->
  snq list
  snq remove <doi>
  snq check <doi>
  snq request <doi|--all> --reward <n>
  snq watch
  snq fetch [<doi>] [--out <dir>] [--wait] [--poll <seconds>]
  snq approve <doi> [--force]

Options:
      --no-wait     Open login browser without waiting for authentication
  -h, --help       Print help
  -V, --version    Print version
"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_waits_by_default_and_accepts_no_wait() {
        assert!(parse_login(std::iter::empty()).unwrap().wait);
        assert!(
            !parse_login(["--no-wait"].into_iter().map(str::to_string))
                .unwrap()
                .wait
        );
        assert!(parse_login(["--bad"].into_iter().map(str::to_string)).is_err());
    }

    #[test]
    fn request_accepts_doi_and_defaults_reward_to_one() {
        let args =
            parse_request(["10.1287/mnsc.2024.05040"].into_iter().map(str::to_string)).unwrap();

        assert_eq!(args.doi.as_deref(), Some("10.1287/mnsc.2024.05040"));
        assert_eq!(args.reward, 1);
        assert!(!args.all);
    }

    #[test]
    fn request_accepts_all_and_reward_flags() {
        let args =
            parse_request(["--all", "--reward", "3"].into_iter().map(str::to_string)).unwrap();

        assert!(args.doi.is_none());
        assert_eq!(args.reward, 3);
        assert!(args.all);

        assert_eq!(
            parse_request(
                ["10.1287/mnsc.2024.05040", "-r", "2"]
                    .into_iter()
                    .map(str::to_string)
            )
            .unwrap()
            .reward,
            2,
        );
    }

    #[test]
    fn request_rejects_missing_invalid_and_ambiguous_values() {
        assert!(parse_request(std::iter::empty()).is_err());
        assert!(parse_request(["--all", "--reward", "0"].into_iter().map(str::to_string)).is_err());
        assert!(parse_request(["--all", "--reward"].into_iter().map(str::to_string)).is_err());
        assert!(parse_request(["--foo"].into_iter().map(str::to_string)).is_err());
        assert!(
            parse_request(
                ["--all", "10.1287/mnsc.2024.05040"]
                    .into_iter()
                    .map(str::to_string)
            )
            .is_err()
        );
    }

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
    fn remote_working_state_promotes_requested_queue_entry() {
        let dir = std::env::temp_dir().join(format!("snq-watch-test-{}", std::process::id()));
        let path = dir.join("queue.jsonl");
        let queue = Queue::new(path);

        queue.add("10.1287/mnsc.2024.05040").unwrap();
        queue
            .set_status("10.1287/mnsc.2024.05040", QueueStatus::Requested)
            .unwrap();

        let status = update_status_from_remote(
            &queue,
            QueueStatus::Requested,
            "10.1287/mnsc.2024.05040",
            RequestRemoteState::Working,
        )
        .unwrap();
        let entries = queue.list().unwrap();

        assert_eq!(status, QueueStatus::Working);
        assert_eq!(entries[0].status, QueueStatus::Working);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn request_all_targets_only_queued_entries() {
        let dir = std::env::temp_dir().join(format!("snq-request-test-{}", std::process::id()));
        let path = dir.join("queue.jsonl");
        let queue = Queue::new(path);

        queue.add("10.1287/mnsc.2024.05040").unwrap();
        queue.add("10.1093/rfs/hhaa075").unwrap();
        queue
            .set_status("10.1093/rfs/hhaa075", QueueStatus::Requested)
            .unwrap();

        let request = RequestArgs {
            doi: None,
            reward: 1,
            all: true,
        };

        assert_eq!(
            request_dois(&queue, &request).unwrap(),
            vec!["10.1287/mnsc.2024.05040".to_string()]
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn approve_requires_doi_and_accepts_force() {
        let args = parse_approve(
            ["10.1287/mnsc.2024.05040", "--force"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();

        assert_eq!(args.doi, "10.1287/mnsc.2024.05040");
        assert!(args.force);
        assert!(parse_approve(std::iter::empty()).is_err());
        assert!(parse_approve(["--bad"].into_iter().map(str::to_string)).is_err());
    }

    #[test]
    fn approve_requires_fetched_status_unless_forced() {
        let dir = std::env::temp_dir().join(format!("snq-approve-test-{}", std::process::id()));
        let path = dir.join("queue.jsonl");
        let queue = Queue::new(path);

        queue.add("10.1287/mnsc.2024.05040").unwrap();
        assert!(ensure_can_approve(&queue, "10.1287/mnsc.2024.05040", false).is_err());

        queue
            .set_status("10.1287/mnsc.2024.05040", QueueStatus::Fetched)
            .unwrap();
        assert!(ensure_can_approve(&queue, "10.1287/mnsc.2024.05040", false).is_ok());
        assert!(ensure_can_approve(&queue, "10.1093/rfs/hhaa075", true).is_ok());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn out_dir_defaults_to_papers() {
        let args = parse_fetch(std::iter::empty()).unwrap();

        assert_eq!(args.out_dir, std::path::PathBuf::from("papers"));
        assert!(!args.wait);
        assert_eq!(args.poll_secs, 30);
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
    fn fetch_accepts_wait_and_poll_interval() {
        let args = parse_fetch(["--wait", "--poll", "5"].into_iter().map(str::to_string)).unwrap();

        assert!(args.wait);
        assert_eq!(args.poll_secs, 5);
    }

    #[test]
    fn fetch_rejects_invalid_poll_interval() {
        assert!(parse_fetch(["--poll"].into_iter().map(str::to_string)).is_err());
        assert!(parse_fetch(["--poll", "0"].into_iter().map(str::to_string)).is_err());
        assert!(parse_fetch(["--poll", "soon"].into_iter().map(str::to_string)).is_err());
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
