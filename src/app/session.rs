use std::env;

use crate::args::DiagnosticArgs;
use crate::browser::profile_dir;
use crate::output::{BalanceOutput, SessionOutput, print_json};
use crate::page::connect_page_session;
use crate::queue::default_queue_path;
use crate::scinet::{SCINET_URL, probe_session};

pub(super) fn handle_session(diagnostic: DiagnosticArgs) -> Result<(), String> {
    let mut output = read_session(!diagnostic.json)?;

    if diagnostic.redact {
        output = redact_session_output(output);
    }

    if diagnostic.json {
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
    if let Some(token_balance) = output.token_balance {
        println!("tokens {token_balance}");
    }

    Ok(())
}

pub(super) fn handle_balance(json: bool) -> Result<(), String> {
    let output = read_session(!json)?;

    if !output.logged_in {
        return Err("balance: not logged into Sci-Net; run `snq login` first".to_string());
    }

    let Some(token_balance) = output.token_balance else {
        return Err("balance: could not determine visible Sci-Net token balance".to_string());
    };

    let output = BalanceOutput {
        logged_in: true,
        token_balance,
    };

    if json {
        print_json(&output)?;
    } else {
        println!("tokens {}", output.token_balance);
    }

    Ok(())
}

fn read_session(interactive: bool) -> Result<SessionOutput, String> {
    let browser = super::detect_browser_or_prompt(interactive)?;
    let profile_dir = profile_dir(&browser).map_err(|error| error.to_string())?;
    let session_browser = browser
        .launch_session(&profile_dir, true)
        .map_err(|error| error.to_string())?;
    let mut page = connect_page_session(browser.engine, session_browser.port())
        .map_err(|error| error.to_string())?;
    let probe = probe_session(&mut page, SCINET_URL).map_err(|error| error.to_string())?;
    let logged_in = probe.is_logged_in();

    Ok(SessionOutput {
        browser: browser.path.display().to_string(),
        engine: browser.engine.to_string(),
        profile: profile_dir.display().to_string(),
        queue: default_queue_path().display().to_string(),
        url: probe.url,
        title: probe.title,
        logged_in,
        token_balance: if logged_in { probe.token_balance } else { None },
    })
}

fn redact_session_output(mut output: SessionOutput) -> SessionOutput {
    output.browser = redact_path_text(&output.browser);
    output.profile = redact_path_text(&output.profile);
    output.queue = redact_path_text(&output.queue);
    output.url = redact_url(&output.url);
    output.token_balance = None;
    output
}

fn redact_path_text(value: &str) -> String {
    let Some(home) = home_dir_text() else {
        return value.to_string();
    };

    value
        .strip_prefix(&home)
        .map(|suffix| format!("~{suffix}"))
        .unwrap_or_else(|| value.to_string())
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

    fn sample_session_output() -> SessionOutput {
        SessionOutput {
            browser: "/Users/example/Browser.app".to_string(),
            engine: "chromium".to_string(),
            profile: "/Users/example/project/.snq/profile".to_string(),
            queue: "/Users/example/project/.snq/queue.jsonl".to_string(),
            url: "https://sci-net.xyz/?token=secret#fragment".to_string(),
            title: "Sci-Net".to_string(),
            logged_in: true,
            token_balance: Some(8),
        }
    }

    #[test]
    fn session_redaction_hides_url_details_and_tokens() {
        let output = redact_session_output(sample_session_output());

        assert_eq!(output.url, "https://sci-net.xyz/");
        assert_eq!(output.token_balance, None);
    }
}
