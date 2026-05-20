use serde::Serialize;

use crate::browser::{BrowserEngine, detect_browser, profile_dir};
use crate::queue::{Queue, default_queue_path};
use crate::scinet::{SCINET_URL, probe_session};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Serialize)]
pub(crate) struct DoctorReport {
    version: String,
    scinet_url: String,
    browser: DoctorBrowser,
    profile: DoctorProfile,
    queue: DoctorQueue,
    session: DoctorSession,
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

                if browser.engine != BrowserEngine::Chromium {
                    (
                        profile_info,
                        DoctorSession {
                            ok: false,
                            logged_in: None,
                            url: None,
                            title: None,
                            message: format!(
                                "session probe is not implemented for {} yet",
                                browser.engine
                            ),
                        },
                    )
                } else {
                    let session_info = match browser.launch_cdp(&path) {
                        Ok(cdp_browser) => match probe_session(cdp_browser.port(), SCINET_URL) {
                            Ok(probe) => {
                                let logged_in = probe.is_logged_in();

                                DoctorSession {
                                    ok: logged_in,
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
                            Err(error) => DoctorSession {
                                ok: false,
                                logged_in: None,
                                url: None,
                                title: None,
                                message: error.to_string(),
                            },
                        },
                        Err(error) => DoctorSession {
                            ok: false,
                            logged_in: None,
                            url: None,
                            title: None,
                            message: error.to_string(),
                        },
                    };

                    (profile_info, session_info)
                }
            }
            Err(error) => (
                DoctorProfile {
                    ok: false,
                    path: None,
                    message: error.to_string(),
                },
                DoctorSession {
                    ok: false,
                    logged_in: None,
                    url: None,
                    title: None,
                    message: "skipped; profile path unavailable".to_string(),
                },
            ),
        },
        Err(_) => (
            DoctorProfile {
                ok: false,
                path: None,
                message: "skipped; browser unavailable".to_string(),
            },
            DoctorSession {
                ok: false,
                logged_in: None,
                url: None,
                title: None,
                message: "skipped; browser unavailable".to_string(),
            },
        ),
    };

    DoctorReport {
        version: VERSION.to_string(),
        scinet_url: SCINET_URL.to_string(),
        browser: browser_info,
        profile: profile_info,
        queue: queue_info,
        session: session_info,
    }
}

pub(crate) fn print_doctor_report(report: &DoctorReport) {
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
    if let Some(url) = &report.session.url {
        println!("session_url\t{url}");
    }
}

fn doctor_label(ok: bool) -> &'static str {
    if ok { "ok" } else { "warn" }
}
