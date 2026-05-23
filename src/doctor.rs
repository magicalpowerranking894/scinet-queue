use serde::Serialize;
use std::env;

use crate::browser::{detect_browser, profile_dir};
use crate::page::connect_page_session;
use crate::queue::{Queue, default_queue_path};
use crate::scinet::{SCINET_URL, SessionProbe, probe_session};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Serialize)]
pub(crate) struct DoctorReport {
    ok: bool,
    version: String,
    scinet_url: String,
    browser: DoctorBrowser,
    profile: DoctorProfile,
    queue: DoctorQueue,
    session: DoctorSession,
}

impl DoctorReport {
    pub(crate) fn is_ok(&self) -> bool {
        self.ok
    }

    pub(crate) fn redact(mut self) -> Self {
        self.browser.path = self.browser.path.map(|path| redact_path_text(&path));
        self.browser.message = redact_path_text(&self.browser.message);
        self.profile.path = self.profile.path.map(|path| redact_path_text(&path));
        self.profile.message = redact_path_text(&self.profile.message);
        self.queue.path = redact_path_text(&self.queue.path);
        self.queue.message = redact_path_text(&self.queue.message);
        self.session.token_balance = None;
        self.session.url = self.session.url.map(|url| redact_url(&url));
        self.session.message = redact_path_text(&self.session.message);
        self
    }
}

#[derive(Debug, Serialize)]
struct DoctorBrowser {
    ok: bool,
    engine: Option<String>,
    path: Option<String>,
    message: String,
}

#[derive(Debug, Serialize)]
struct DoctorProfile {
    ok: bool,
    path: Option<String>,
    message: String,
}

#[derive(Debug, Serialize)]
struct DoctorQueue {
    ok: bool,
    path: String,
    entries: Option<usize>,
    message: String,
}

#[derive(Debug, Serialize)]
struct DoctorSession {
    ok: bool,
    phase: String,
    logged_in: Option<bool>,
    token_balance: Option<u32>,
    url: Option<String>,
    title: Option<String>,
    message: String,
}

pub(crate) fn doctor_report(queue: &Queue) -> DoctorReport {
    let queue_path = default_queue_path();
    let queue_info = match queue.list() {
        Ok(entries) => DoctorQueue {
            ok: true,
            path: queue_path.display().to_string(),
            entries: Some(entries.len()),
            message: "readable".to_string(),
        },
        Err(error) => DoctorQueue {
            ok: false,
            path: queue_path.display().to_string(),
            entries: None,
            message: error.to_string(),
        },
    };

    let browser_result = detect_browser();
    let browser_info = match &browser_result {
        Ok(browser) => DoctorBrowser {
            ok: true,
            engine: Some(browser.engine.to_string()),
            path: Some(browser.path.display().to_string()),
            message: "found".to_string(),
        },
        Err(error) => DoctorBrowser {
            ok: false,
            engine: None,
            path: None,
            message: error.to_string(),
        },
    };

    let (profile_info, session_info) = match browser_result {
        Ok(browser) => match profile_dir(&browser) {
            Ok(path) => {
                let profile_info = DoctorProfile {
                    ok: true,
                    path: Some(path.display().to_string()),
                    message: "resolved".to_string(),
                };

                let session_info = match browser.launch_session(&path, true) {
                    Ok(session_browser) => {
                        match connect_page_session(browser.engine, session_browser.port()) {
                            Ok(mut page) => match probe_session(&mut page, SCINET_URL) {
                                Ok(probe) => session_from_probe(probe),
                                Err(error) => session_failure("probe", error),
                            },
                            Err(error) => session_failure("connect", error),
                        }
                    }
                    Err(error) => session_failure("launch", error),
                };

                (profile_info, session_info)
            }
            Err(error) => (
                DoctorProfile {
                    ok: false,
                    path: None,
                    message: error.to_string(),
                },
                session_skipped("skipped; profile path unavailable"),
            ),
        },
        Err(_) => (
            DoctorProfile {
                ok: false,
                path: None,
                message: "skipped; browser unavailable".to_string(),
            },
            session_skipped("skipped; browser unavailable"),
        ),
    };

    let ok = browser_info.ok && profile_info.ok && queue_info.ok && session_info.ok;

    DoctorReport {
        ok,
        version: VERSION.to_string(),
        scinet_url: SCINET_URL.to_string(),
        browser: browser_info,
        profile: profile_info,
        queue: queue_info,
        session: session_info,
    }
}

pub(crate) fn print_doctor_report(report: &DoctorReport) {
    println!("ok\t{}", report.ok);
    println!("version\t{}", report.version);
    println!("scinet\t{}", report.scinet_url);
    println!(
        "browser\t{}\t{}",
        doctor_label(report.browser.ok),
        report.browser.message
    );
    if let Some(path) = &report.browser.path {
        println!("browser_path\t{path}");
    }
    if let Some(engine) = &report.browser.engine {
        println!("browser_engine\t{engine}");
    }
    println!(
        "profile\t{}\t{}",
        doctor_label(report.profile.ok),
        report.profile.message
    );
    if let Some(path) = &report.profile.path {
        println!("profile_path\t{path}");
    }
    println!(
        "queue\t{}\t{}\t{} entries\t{}",
        doctor_label(report.queue.ok),
        report.queue.path,
        report.queue.entries.unwrap_or(0),
        report.queue.message
    );
    println!(
        "session\t{}\t{}",
        doctor_label(report.session.ok),
        report.session.message
    );
    println!("session_phase\t{}", report.session.phase);
    if let Some(url) = &report.session.url {
        println!("session_url\t{url}");
    }
    if report.session.logged_in == Some(true) {
        match report.session.token_balance {
            Some(token_balance) => println!("session_tokens\t{token_balance}"),
            None => println!("session_tokens\tunknown"),
        }
    }
}

fn session_from_probe(probe: SessionProbe) -> DoctorSession {
    let logged_in = probe.is_logged_in();
    let token_balance = if logged_in { probe.token_balance } else { None };
    let (ok, phase, message) = match (logged_in, token_balance) {
        (true, Some(_)) => (true, "authenticated", "logged in"),
        (true, None) => (false, "balance", "logged in; token balance unknown"),
        (false, _) => (
            false,
            "auth",
            "not logged into Sci-Net; run `snq login` first",
        ),
    };

    DoctorSession {
        ok,
        phase: phase.to_string(),
        logged_in: Some(logged_in),
        token_balance,
        url: Some(probe.url),
        title: Some(probe.title),
        message: message.to_string(),
    }
}

fn session_failure(phase: &str, error: impl ToString) -> DoctorSession {
    DoctorSession {
        ok: false,
        phase: phase.to_string(),
        logged_in: None,
        token_balance: None,
        url: None,
        title: None,
        message: format!("{phase} failed: {}", error.to_string()),
    }
}

fn session_skipped(message: &str) -> DoctorSession {
    DoctorSession {
        ok: false,
        phase: "skipped".to_string(),
        logged_in: None,
        token_balance: None,
        url: None,
        title: None,
        message: message.to_string(),
    }
}

fn doctor_label(ok: bool) -> &'static str {
    if ok { "ok" } else { "warn" }
}

fn redact_path_text(value: &str) -> String {
    let Some(home) = home_dir_text() else {
        return value.to_string();
    };

    value.replace(&home, "~")
}

fn home_dir_text() -> Option<String> {
    ["HOME", "USERPROFILE"]
        .into_iter()
        .filter_map(env::var_os)
        .map(std::path::PathBuf::from)
        .find(|path| !path.as_os_str().is_empty())
        .map(|path| path.display().to_string())
}

fn redact_url(value: &str) -> String {
    value.split(['?', '#']).next().unwrap_or(value).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_failure_reports_phase() {
        let session = session_failure("connect", "websocket refused");

        assert!(!session.ok);
        assert_eq!(session.phase, "connect");
        assert_eq!(session.message, "connect failed: websocket refused");
    }

    #[test]
    fn session_probe_reports_auth_phase() {
        let session = session_from_probe(SessionProbe {
            title: "Sci-Net".to_string(),
            url: "https://sci-net.xyz/".to_string(),
            text: "scientific communication support network No account yet?".to_string(),
            token_balance: None,
        });

        assert!(!session.ok);
        assert_eq!(session.phase, "auth");
        assert_eq!(session.logged_in, Some(false));
        assert_eq!(
            session.message,
            "not logged into Sci-Net; run `snq login` first"
        );
    }

    #[test]
    fn session_probe_warns_when_token_balance_is_unknown() {
        let session = session_from_probe(SessionProbe {
            title: "Sci-Net".to_string(),
            url: "https://sci-net.xyz/".to_string(),
            text: "tokens request library".to_string(),
            token_balance: None,
        });

        assert!(!session.ok);
        assert_eq!(session.phase, "balance");
        assert_eq!(session.logged_in, Some(true));
        assert_eq!(session.message, "logged in; token balance unknown");
    }

    #[test]
    fn report_redaction_hides_home_paths_tokens_and_url_details() {
        let home = home_dir_text().unwrap_or_else(|| "/tmp/snq-home".to_string());
        let report = DoctorReport {
            ok: true,
            version: VERSION.to_string(),
            scinet_url: SCINET_URL.to_string(),
            browser: DoctorBrowser {
                ok: true,
                engine: Some("chromium".to_string()),
                path: Some(format!("{home}/Applications/Browser")),
                message: "found".to_string(),
            },
            profile: DoctorProfile {
                ok: true,
                path: Some(format!("{home}/.local/state/scinet-queue/browser/chromium")),
                message: "resolved".to_string(),
            },
            queue: DoctorQueue {
                ok: true,
                path: format!("{home}/work/.snq/queue.jsonl"),
                entries: Some(1),
                message: format!("readable at {home}/work/.snq/queue.jsonl"),
            },
            session: DoctorSession {
                ok: true,
                phase: "authenticated".to_string(),
                logged_in: Some(true),
                token_balance: Some(42),
                url: Some("https://sci-net.xyz/?token=secret#fragment".to_string()),
                title: Some("Sci-Net".to_string()),
                message: format!("logged in with {home}/profile"),
            },
        }
        .redact();

        assert_eq!(
            report.browser.path.as_deref(),
            Some("~/Applications/Browser")
        );
        assert_eq!(
            report.profile.path.as_deref(),
            Some("~/.local/state/scinet-queue/browser/chromium")
        );
        assert_eq!(report.queue.path, "~/work/.snq/queue.jsonl");
        assert!(report.queue.message.contains("~/work/.snq/queue.jsonl"));
        assert_eq!(report.session.token_balance, None);
        assert_eq!(report.session.url.as_deref(), Some("https://sci-net.xyz/"));
        assert!(report.session.message.contains("~/profile"));
    }
}
