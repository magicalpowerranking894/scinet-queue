use serde::Serialize;

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
        Ok(browser) => match profile_dir(browser.engine) {
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
}

fn session_from_probe(probe: SessionProbe) -> DoctorSession {
    let logged_in = probe.is_logged_in();

    DoctorSession {
        ok: logged_in,
        phase: if logged_in { "authenticated" } else { "auth" }.to_string(),
        logged_in: Some(logged_in),
        url: Some(probe.url),
        title: Some(probe.title),
        message: if logged_in {
            "logged in".to_string()
        } else {
            "not logged in; run `snq login`".to_string()
        },
    }
}

fn session_failure(phase: &str, error: impl ToString) -> DoctorSession {
    DoctorSession {
        ok: false,
        phase: phase.to_string(),
        logged_in: None,
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
        url: None,
        title: None,
        message: message.to_string(),
    }
}

fn doctor_label(ok: bool) -> &'static str {
    if ok { "ok" } else { "warn" }
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
        });

        assert!(!session.ok);
        assert_eq!(session.phase, "auth");
        assert_eq!(session.logged_in, Some(false));
    }
}
