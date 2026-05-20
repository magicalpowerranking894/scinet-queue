use std::env;
use std::thread;
use std::time::Duration;

use crate::args::{
    RequestArgs, parse_approve, parse_fetch, parse_json_flag, parse_login, parse_request,
    parse_view,
};
use crate::browser::{detect_browser, profile_dir};
use crate::cdp;
use crate::doctor::{doctor_report, print_doctor_report};
use crate::output::{
    RequestOutput, SessionOutput, ViewOutput, WatchOutput, compact_text, format_response,
    print_help, print_json,
};
use crate::papers::{extract_dois, fetch_dois, fetch_one, read_import_text};
use crate::queue::{
    AddResult, Queue, QueueStatus, RemoveResult, StatusResult, default_queue_path, normalize_doi,
};
use crate::scinet::{
    RequestRemoteState, RequestView, SCINET_URL, ScinetResponse, probe_current_session,
    probe_session, request_doi, search_doi, view_request,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let queue = Queue::new(default_queue_path());

    match args.next().as_deref() {
        None | Some("-h" | "--help" | "help") => {
            print_help(VERSION);
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

            if let Some(extra) = args.next() {
                return Err(format!("import: unexpected argument `{extra}`"));
            }

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
            let json = parse_json_flag("list", args)?;
            let entries = queue.list().map_err(|error| error.to_string())?;

            if json {
                print_json(&entries)?;
                return Ok(());
            }

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

            if let Some(extra) = args.next() {
                return Err(format!("remove: unexpected argument `{extra}`"));
            }

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
            let json = parse_json_flag("session", args)?;
            let browser = detect_browser().map_err(|error| error.to_string())?;
            let profile_dir = profile_dir(browser.engine).map_err(|error| error.to_string())?;
            let cdp_browser = browser
                .launch_cdp(&profile_dir)
                .map_err(|error| error.to_string())?;
            let probe =
                probe_session(cdp_browser.port(), SCINET_URL).map_err(|error| error.to_string())?;
            let logged_in = probe.is_logged_in();

            let output = SessionOutput {
                browser: browser.path.display().to_string(),
                engine: browser.engine.to_string(),
                profile: profile_dir.display().to_string(),
                queue: default_queue_path().display().to_string(),
                url: probe.url,
                title: probe.title,
                logged_in,
            };

            if json {
                print_json(&output)?;
                return Ok(());
            }

            println!("browser {}", output.browser);
            println!("engine {}", output.engine);
            println!("profile {}", output.profile);
            println!("queue {}", output.queue);
            println!("url {}", output.url);
            println!("title {}", output.title);
            println!(
                "login {}",
                if output.logged_in {
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

            if let Some(extra) = args.next() {
                return Err(format!("check: unexpected argument `{extra}`"));
            }

            let doi = normalize_doi(&doi).map_err(|error| error.to_string())?;
            let response = with_scinet_session(|port| search_doi(port, SCINET_URL, &doi))?;
            let json = format_response(&response)?;

            println!("{json}");
        }
        Some("request") => {
            let request = parse_request(args)?;
            let dois = request_dois(&queue, &request)?;

            if dois.is_empty() {
                if request.json {
                    print_json(&Vec::<RequestOutput>::new())?;
                    return Ok(());
                }

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

                    ensure_request_ok(doi, &response)?;

                    mark_requested(&queue, doi)?;
                    responses.push(response);
                }

                Ok(responses)
            })?;

            if request.json {
                let output = dois
                    .iter()
                    .zip(responses.iter())
                    .map(|(doi, response)| RequestOutput {
                        doi: doi.clone(),
                        response: response.clone(),
                    })
                    .collect::<Vec<_>>();
                print_json(&output)?;
                return Ok(());
            }

            for (doi, response) in dois.iter().zip(responses.iter()) {
                if request.all {
                    println!("requested\t{doi}");
                } else {
                    println!("{}", format_response(response)?);
                }
            }
        }
        Some("watch") => {
            let json = parse_json_flag("watch", args)?;
            let entries = queue.list().map_err(|error| error.to_string())?;

            if entries.is_empty() {
                if json {
                    print_json(&Vec::<WatchOutput>::new())?;
                } else {
                    println!("queue empty");
                }
                return Ok(());
            }

            let views = with_scinet_views(entries.iter().map(|entry| entry.doi.as_str()))?;
            let mut output = Vec::new();

            for (entry, view) in entries.iter().zip(views.iter()) {
                let remote_state = view.remote_state();
                let status =
                    update_status_from_remote(&queue, entry.status, &entry.doi, remote_state)?;

                output.push(WatchOutput {
                    doi: entry.doi.clone(),
                    status,
                    remote_state,
                });
            }

            if json {
                print_json(&output)?;
            } else {
                for row in output {
                    let status = row.status;
                    let remote_state = row.remote_state;
                    let doi = row.doi;

                    println!("{}\t{}\t{}", status, remote_state.as_str(), doi);
                }
            }
        }
        Some("view") => {
            let view_args = parse_view(args)?;
            let mut views = with_scinet_views(std::iter::once(view_args.doi.as_str()))?;
            let view = views.remove(0);
            let state = view.remote_state();
            let output = ViewOutput {
                url: view.url,
                title: view.title,
                state,
                pdf_urls: view.pdf_urls,
                text: view.text,
            };

            if view_args.json {
                print_json(&output)?;
                return Ok(());
            }

            println!("url\t{}", output.url);
            println!("title\t{}", output.title);
            println!("state\t{}", output.state.as_str());
            println!("pdfs\t{}", output.pdf_urls.len());

            for pdf_url in &output.pdf_urls {
                println!("pdf\t{pdf_url}");
            }

            println!("text\t{}", compact_text(&output.text));
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
            mark_approved(&queue, &doi)?;

            println!("approved\t{doi}");
        }
        Some("doctor") => {
            let json = parse_json_flag("doctor", args)?;
            let report = doctor_report(&queue);

            if json {
                print_json(&report)?;
            } else {
                print_doctor_report(&report);
            }
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

fn ensure_request_ok(doi: &str, response: &ScinetResponse) -> Result<(), String> {
    if response.ok {
        Ok(())
    } else {
        Err(format!(
            "request: Sci-Net returned status {} for {doi}",
            response.status
        ))
    }
}

fn mark_approved(queue: &Queue, doi: &str) -> Result<(), String> {
    match queue
        .set_status(doi, QueueStatus::Approved)
        .map_err(|error| error.to_string())?
    {
        StatusResult::Updated(_) => {}
        StatusResult::NotFound(_) => {
            let _ = queue.add(doi).map_err(|error| error.to_string())?;
            let _ = queue
                .set_status(doi, QueueStatus::Approved)
                .map_err(|error| error.to_string())?;
        }
    }

    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
            json: false,
        };

        assert_eq!(
            request_dois(&queue, &request).unwrap(),
            vec!["10.1287/mnsc.2024.05040".to_string()]
        );

        let _ = fs::remove_dir_all(dir);
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
    fn forced_approve_creates_local_state_when_missing() {
        let dir =
            std::env::temp_dir().join(format!("snq-force-approve-test-{}", std::process::id()));
        let path = dir.join("queue.jsonl");
        let queue = Queue::new(path);
        let doi = "10.1093/rfs/hhaa075";

        ensure_can_approve(&queue, doi, true).unwrap();
        mark_approved(&queue, doi).unwrap();

        let entries = queue.list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].doi, doi);
        assert_eq!(entries[0].status, QueueStatus::Approved);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn request_non_ok_response_is_rejected() {
        let response = ScinetResponse {
            ok: false,
            status: 500,
            body: serde_json::json!({ "error": "boom" }),
        };

        assert!(ensure_request_ok("10.1093/rfs/hhaa075", &response).is_err());
    }
}
