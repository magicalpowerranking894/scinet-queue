use std::env;
use std::io::{self, IsTerminal, Write};
use std::thread;
use std::time::Duration;

use crate::args::{
    BrowsersAction, RequestArgs, parse_approve, parse_browsers, parse_fetch, parse_json_flag,
    parse_login, parse_request, parse_view,
};
use crate::browser::{
    BROWSER_ENV, Browser, BrowserChoice, BrowserError, available_browser_candidates,
    browser_choices, browser_from_path, browser_preference_exists, browser_preference_path,
    clear_browser_preference, detect_browser, profile_dir, save_browser_preference,
};
use crate::doctor::{doctor_report, print_doctor_report};
use crate::output::{
    ApproveOutput, BrowserChoiceOutput, BrowserListOutput, FetchOutput, FetchOutputStatus,
    RequestOutput, SessionOutput, ViewOutput, WatchOutput, compact_text, format_response,
    print_help, print_json,
};
use crate::page::{BrowserPageSession, PageError, PageSession, connect_page_session};
use crate::papers::{
    FetchResult, extract_dois, fetch_dois, fetch_one, read_import_text, update_status_from_remote,
};
use crate::queue::{
    AddResult, Queue, QueueStatus, RemoveResult, StatusResult, default_queue_path, normalize_doi,
};
use crate::scinet::{
    RequestRemoteState, RequestView, SCINET_URL, ScinetResponse, probe_current_session,
    probe_session, request_doi, search_doi, view_request,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn args_want_json(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--json")
}

pub fn run(args: Vec<String>) -> Result<(), String> {
    if args
        .get(1)
        .map(|arg| matches!(arg.as_str(), "-h" | "--help" | "help"))
        .unwrap_or(false)
    {
        print_help(VERSION);
        return Ok(());
    }

    let mut args = args.into_iter();
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
            let browser = detect_browser_or_prompt(true)?;
            let profile_dir = profile_dir(browser.engine).map_err(|error| error.to_string())?;

            if login.wait {
                let session_browser = browser
                    .launch_session(&profile_dir, false)
                    .map_err(|error| error.to_string())?;

                println!("opened {}", browser.engine);
                println!("profile {}", profile_dir.display());
                println!("waiting for Sci-Net login; press Ctrl-C to cancel");

                wait_for_login(browser.engine, session_browser.port())?;
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
            let browser = detect_browser_or_prompt(!json)?;
            let profile_dir = profile_dir(browser.engine).map_err(|error| error.to_string())?;
            let session_browser = browser
                .launch_session(&profile_dir, true)
                .map_err(|error| error.to_string())?;
            let mut page = connect_page_session(browser.engine, session_browser.port())
                .map_err(|error| error.to_string())?;
            let probe = probe_session(&mut page, SCINET_URL).map_err(|error| error.to_string())?;
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
        Some("browsers") => {
            let browser_args = parse_browsers(args)?;
            let print_list = !matches!(&browser_args.action, BrowsersAction::Clear);

            match browser_args.action {
                BrowsersAction::List => {}
                BrowsersAction::Pick => {
                    let browser = prompt_browser_choice()?;
                    eprintln!(
                        "saved browser preference {}",
                        browser_preference_path().display()
                    );
                    eprintln!("selected {} {}", browser.engine, browser.path.display());
                }
                BrowsersAction::Set(path) => {
                    let browser = browser_from_path(path).map_err(|error| error.to_string())?;
                    save_browser_preference(&browser).map_err(|error| error.to_string())?;

                    if !browser_args.json {
                        println!("browser preference saved");
                    }
                }
                BrowsersAction::Clear => {
                    let removed = clear_browser_preference().map_err(|error| error.to_string())?;

                    if !browser_args.json {
                        if removed {
                            println!("browser preference cleared");
                        } else {
                            println!("browser preference not set");
                        }
                    }
                }
            }

            let output = browser_list_output();

            if browser_args.json {
                print_json(&output)?;
                return Ok(());
            }

            if print_list {
                print_browser_list(&output);
            }
        }
        Some("check") => {
            let mut doi = None;

            for arg in args {
                match arg.as_str() {
                    "--json" => {}
                    value if value.starts_with('-') => {
                        return Err(format!("check: unknown option `{value}`"));
                    }
                    value => {
                        if doi.is_some() {
                            return Err(format!("check: unexpected argument `{value}`"));
                        }

                        doi = Some(normalize_doi(value).map_err(|error| error.to_string())?);
                    }
                }
            }

            let Some(doi) = doi else {
                return Err("check: missing DOI".to_string());
            };
            let response = with_scinet_session(false, |page| search_doi(page, SCINET_URL, &doi))?;
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

            let responses = with_scinet_page(!request.json, |page| {
                let mut responses = Vec::new();

                for doi in &dois {
                    let response = request_doi(page, SCINET_URL, doi, request.reward)
                        .map_err(|error| error.to_string())?;

                    if response.looks_logged_out() {
                        return Err("not logged into Sci-Net; run `snq login` first".to_string());
                    }

                    record_successful_request(&queue, doi, &response)?;
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

            let views = with_scinet_views(entries.iter().map(|entry| entry.doi.as_str()), !json)?;
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
            let mut views =
                with_scinet_views(std::iter::once(view_args.doi.as_str()), !view_args.json)?;
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
                if fetch.json {
                    print_json(&Vec::<FetchOutput>::new())?;
                    return Ok(());
                }

                println!("queue empty");
                return Ok(());
            }

            let outputs = with_scinet_page(!fetch.json, |page| {
                loop {
                    let mut outputs = Vec::new();
                    let mut fetched_any = false;

                    for doi in &dois {
                        match fetch_one(&queue, page, doi, &fetch.out_dir) {
                            Ok(FetchResult::Fetched(path)) => {
                                fetched_any = true;
                                outputs.push(FetchOutput {
                                    doi: doi.clone(),
                                    status: FetchOutputStatus::Fetched,
                                    remote_state: RequestRemoteState::Pdf,
                                    path: Some(path.display().to_string()),
                                });
                            }
                            Ok(FetchResult::NoPdf(remote_state)) => outputs.push(FetchOutput {
                                doi: doi.clone(),
                                status: FetchOutputStatus::NoPdf,
                                remote_state,
                                path: None,
                            }),
                            Err(error) => return Err(error),
                        }
                    }

                    if fetched_any || !fetch.wait {
                        return Ok(outputs);
                    }

                    if fetch.json {
                        eprintln!("no PDFs available; waiting {}s", fetch.poll_secs);
                    } else {
                        println!("no PDFs available; waiting {}s", fetch.poll_secs);
                    }
                    thread::sleep(Duration::from_secs(fetch.poll_secs));
                }
            })?;

            if fetch.json {
                print_json(&outputs)?;
            } else if outputs
                .iter()
                .all(|output| output.path.as_deref().is_none())
            {
                for output in outputs {
                    println!("no-pdf\t{}\t{}", output.remote_state.as_str(), output.doi);
                }
            } else {
                for output in outputs {
                    if let Some(path) = output.path {
                        println!("{path}");
                    }
                }
            }
        }
        Some("approve") => {
            let approve = parse_approve(args)?;
            let doi = approve.doi.clone();
            ensure_can_approve(&queue, &doi, approve.force)?;
            mark_approved(&queue, &doi)?;

            if approve.json {
                print_json(&ApproveOutput {
                    doi,
                    status: QueueStatus::Approved,
                    forced: approve.force,
                })?;
                return Ok(());
            }
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

            if !report.is_ok() {
                return Err("doctor: checks failed".to_string());
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

fn with_scinet_session<F>(interactive: bool, operation: F) -> Result<ScinetResponse, String>
where
    F: FnOnce(&mut BrowserPageSession) -> Result<ScinetResponse, PageError>,
{
    with_scinet_page(interactive, |page| {
        let response = operation(page).map_err(|error| error.to_string())?;

        if response.looks_logged_out() {
            return Err("not logged into Sci-Net; run `snq login` first".to_string());
        }

        Ok(response)
    })
}

fn browser_list_output() -> BrowserListOutput {
    let browsers = browser_choices()
        .into_iter()
        .map(browser_choice_output)
        .collect::<Vec<_>>();
    let selected = browsers.iter().find(|browser| browser.selected).cloned();

    BrowserListOutput {
        override_env: BROWSER_ENV.to_string(),
        preference_path: browser_preference_path().display().to_string(),
        selected,
        browsers,
    }
}

fn browser_choice_output(choice: BrowserChoice) -> BrowserChoiceOutput {
    BrowserChoiceOutput {
        selected: choice.selected,
        available: choice.available,
        engine: choice.engine.to_string(),
        source: choice.source.to_string(),
        path: choice.path.display().to_string(),
    }
}

fn print_browser_list(output: &BrowserListOutput) {
    if output.browsers.is_empty() {
        println!("no supported browsers found");
        println!("override {BROWSER_ENV}=/path/to/browser");
        return;
    }

    for browser in &output.browsers {
        let marker = if browser.selected { "*" } else { " " };
        let availability = if browser.available {
            "available"
        } else {
            "missing"
        };

        println!(
            "{marker}\t{}\t{}\t{}\t{}",
            browser.engine, browser.source, availability, browser.path
        );
    }

    println!("preference {}", browser_preference_path().display());
    println!("override {BROWSER_ENV}=/path/to/browser");
}

fn detect_browser_or_prompt(interactive: bool) -> Result<Browser, String> {
    if env::var_os(BROWSER_ENV).is_some() {
        return detect_browser().map_err(|error| error.to_string());
    }

    match detect_browser() {
        Ok(browser) => {
            if interactive && !browser_preference_exists() && can_prompt() {
                let candidates = available_browser_candidates();

                if candidates.len() > 1 {
                    return prompt_browser_choice();
                }
            }

            Ok(browser)
        }
        Err(BrowserError::PreferenceBrowserNotFound(path)) if interactive && can_prompt() => {
            eprintln!("configured browser is missing: {}", path.display());
            prompt_browser_choice()
        }
        Err(error) => Err(error.to_string()),
    }
}

fn can_prompt() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

fn prompt_browser_choice() -> Result<Browser, String> {
    if env::var_os(BROWSER_ENV).is_some() {
        return Err(format!(
            "{BROWSER_ENV} is set; unset it before choosing a saved browser preference"
        ));
    }

    let browsers = available_browser_candidates();

    if browsers.is_empty() {
        return Err("no supported browsers found".to_string());
    }

    if browsers.len() == 1 {
        let browser = browsers[0].clone();
        save_browser_preference(&browser).map_err(|error| error.to_string())?;
        return Ok(browser);
    }

    eprintln!("choose browser for this workspace:");

    for (index, browser) in browsers.iter().enumerate() {
        eprintln!(
            "  {}. {}\t{}",
            index + 1,
            browser.engine,
            browser.path.display()
        );
    }

    loop {
        eprint!("browser [1-{}]: ", browsers.len());
        io::stderr().flush().map_err(|error| error.to_string())?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .map_err(|error| error.to_string())?;

        let selection = input.trim();
        let Ok(index) = selection.parse::<usize>() else {
            eprintln!("enter a number from 1 to {}", browsers.len());
            continue;
        };

        if !(1..=browsers.len()).contains(&index) {
            eprintln!("enter a number from 1 to {}", browsers.len());
            continue;
        }

        let browser = browsers[index - 1].clone();
        save_browser_preference(&browser).map_err(|error| error.to_string())?;
        return Ok(browser);
    }
}

fn with_scinet_page<F, T>(interactive: bool, operation: F) -> Result<T, String>
where
    F: FnOnce(&mut BrowserPageSession) -> Result<T, String>,
{
    let browser = detect_browser_or_prompt(interactive)?;
    let profile_dir = profile_dir(browser.engine).map_err(|error| error.to_string())?;
    let session_browser = browser
        .launch_session(&profile_dir, true)
        .map_err(|error| error.to_string())?;
    let mut page = connect_page_session(browser.engine, session_browser.port())
        .map_err(|error| error.to_string())?;
    let probe = probe_session(&mut page, SCINET_URL).map_err(|error| error.to_string())?;

    if !probe.is_logged_in() {
        return Err("not logged into Sci-Net; run `snq login` first".to_string());
    }

    operation(&mut page)
}

fn with_scinet_views<'a>(
    dois: impl Iterator<Item = &'a str>,
    interactive: bool,
) -> Result<Vec<RequestView>, String> {
    with_scinet_page(interactive, |page| {
        let mut views = Vec::new();

        for doi in dois {
            views.push(view_request(page, SCINET_URL, doi).map_err(|error| error.to_string())?);
        }

        Ok(views)
    })
}

fn wait_for_login(engine: crate::browser::BrowserEngine, port: u16) -> Result<(), String> {
    let mut page = connect_page_session(engine, port).map_err(|error| error.to_string())?;

    page.navigate(SCINET_URL)
        .map_err(|error| error.to_string())?;

    loop {
        let probe = probe_current_session(&mut page).map_err(|error| error.to_string())?;

        if probe.is_logged_in() {
            return Ok(());
        }

        thread::sleep(Duration::from_secs(1));
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
        if let Some(error) = response.logical_error() {
            Err(format!(
                "request: Sci-Net returned logical error for {doi}: {error}"
            ))
        } else {
            Ok(())
        }
    } else {
        Err(format!(
            "request: Sci-Net returned status {} for {doi}",
            response.status
        ))
    }
}

fn record_successful_request(
    queue: &Queue,
    doi: &str,
    response: &ScinetResponse,
) -> Result<(), String> {
    ensure_request_ok(doi, response)?;
    mark_requested(queue, doi)
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

        queue.add("10.1000/snq-example").unwrap();
        queue
            .set_status("10.1000/snq-example", QueueStatus::Requested)
            .unwrap();

        let status = update_status_from_remote(
            &queue,
            QueueStatus::Requested,
            "10.1000/snq-example",
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

        queue.add("10.1000/snq-example").unwrap();
        queue.add("10.1000/snq-alt").unwrap();
        queue
            .set_status("10.1000/snq-alt", QueueStatus::Requested)
            .unwrap();

        let request = RequestArgs {
            doi: None,
            reward: 1,
            all: true,
            json: false,
        };

        assert_eq!(
            request_dois(&queue, &request).unwrap(),
            vec!["10.1000/snq-example".to_string()]
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn successful_request_is_marked_before_later_failure() {
        let dir = std::env::temp_dir().join(format!(
            "snq-request-partial-failure-test-{}",
            std::process::id()
        ));
        let path = dir.join("queue.jsonl");
        let queue = Queue::new(path);

        queue.add("10.1000/snq-example").unwrap();
        queue.add("10.1000/snq-alt").unwrap();

        let ok = ScinetResponse {
            ok: true,
            status: 200,
            body: serde_json::json!({ "success": true }),
        };
        let logical_error = ScinetResponse {
            ok: true,
            status: 200,
            body: serde_json::json!({ "success": false, "message": "invalid request" }),
        };

        record_successful_request(&queue, "10.1000/snq-example", &ok).unwrap();
        assert!(record_successful_request(&queue, "10.1000/snq-alt", &logical_error).is_err());

        let entries = queue.list().unwrap();
        assert_eq!(entries[0].status, QueueStatus::Requested);
        assert_eq!(entries[1].status, QueueStatus::Queued);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn approve_requires_fetched_status_unless_forced() {
        let dir = std::env::temp_dir().join(format!("snq-approve-test-{}", std::process::id()));
        let path = dir.join("queue.jsonl");
        let queue = Queue::new(path);

        queue.add("10.1000/snq-example").unwrap();
        assert!(ensure_can_approve(&queue, "10.1000/snq-example", false).is_err());

        queue
            .set_status("10.1000/snq-example", QueueStatus::Fetched)
            .unwrap();
        assert!(ensure_can_approve(&queue, "10.1000/snq-example", false).is_ok());
        assert!(ensure_can_approve(&queue, "10.1000/snq-alt", true).is_ok());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn forced_approve_creates_local_state_when_missing() {
        let dir =
            std::env::temp_dir().join(format!("snq-force-approve-test-{}", std::process::id()));
        let path = dir.join("queue.jsonl");
        let queue = Queue::new(path);
        let doi = "10.1000/snq-alt";

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

        assert!(ensure_request_ok("10.1000/snq-alt", &response).is_err());
    }

    #[test]
    fn request_logical_error_response_is_rejected() {
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: serde_json::json!({ "success": false, "message": "invalid request" }),
        };

        assert!(ensure_request_ok("10.1000/snq-alt", &response).is_err());
    }
}
