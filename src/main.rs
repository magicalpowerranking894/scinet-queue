mod browser;
mod cdp;
mod queue;

use std::env;
use std::process;

use browser::{SCINET_URL, detect_browser, profile_dir};
use cdp::probe_session;
use queue::{AddResult, Queue, RemoveResult, default_queue_path};

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
        Some("check" | "request" | "watch" | "fetch" | "approve") => {
            return Err("command is scaffolded but not implemented yet".to_string());
        }
        Some(command) => {
            return Err(format!(
                "unknown command `{command}`\nrun `snq help` for usage"
            ));
        }
    }

    Ok(())
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
  snq fetch [--out <dir>]
  snq approve <doi>

Options:
  -h, --help       Print help
  -V, --version    Print version
"
    );
}
