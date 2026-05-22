use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn snq() -> Command {
    Command::new(env!("CARGO_BIN_EXE_snq"))
}

fn temp_workspace(name: &str) -> std::path::PathBuf {
    let path =
        std::env::temp_dir().join(format!("snq-{name}-{}-{}", std::process::id(), unix_time()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_fake_browser(path: &std::path::Path) {
    fs::write(path, "").unwrap();

    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }
}

#[test]
fn help_and_version_work() {
    let help = snq().arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Usage:"));

    let command_help = snq().args(["request", "--help"]).output().unwrap();
    assert!(command_help.status.success());
    assert!(String::from_utf8_lossy(&command_help.stdout).contains("Usage:"));

    let version = snq().arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).starts_with("snq "));
}

#[test]
fn queue_round_trip_works_from_cli() {
    let dir = temp_workspace("queue-round-trip");

    let add = snq()
        .current_dir(&dir)
        .args(["add", "10.1000/snq-example"])
        .output()
        .unwrap();
    assert!(add.status.success());

    let list = snq()
        .current_dir(&dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    assert!(list.status.success());
    let list = String::from_utf8_lossy(&list.stdout);
    assert!(list.contains("\"doi\": \"10.1000/snq-example\""));
    assert!(list.contains("\"status\": \"queued\""));

    let remove = snq()
        .current_dir(&dir)
        .args(["remove", "10.1000/snq-example"])
        .output()
        .unwrap();
    assert!(remove.status.success());

    let list = snq().current_dir(&dir).arg("list").output().unwrap();
    assert!(list.status.success());
    assert_eq!(String::from_utf8_lossy(&list.stdout), "queue empty\n");

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_extra_trailing_arguments() {
    let dir = temp_workspace("extra-args");
    let import = dir.join("papers.md");
    fs::write(&import, "10.1000/snq-example\n").unwrap();

    let import_result = snq()
        .current_dir(&dir)
        .args(["import", import.to_str().unwrap(), "extra"])
        .output()
        .unwrap();
    assert!(!import_result.status.success());
    assert!(String::from_utf8_lossy(&import_result.stderr).contains("unexpected argument"));

    let remove_result = snq()
        .current_dir(&dir)
        .args(["remove", "10.1000/snq-example", "extra"])
        .output()
        .unwrap();
    assert!(!remove_result.status.success());
    assert!(String::from_utf8_lossy(&remove_result.stderr).contains("unexpected argument"));

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn url_prints_scinet_request_url_without_browser() {
    let dir = temp_workspace("url");

    let output = snq()
        .current_dir(&dir)
        .args(["url", "10.1016/s0272-5231(21)01013-3"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "https://sci-net.xyz/10.1016/s0272-5231%2821%2901013-3\n"
    );
    assert!(output.stderr.is_empty());

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn url_preserves_trailing_parenthesis_in_quoted_doi() {
    let dir = temp_workspace("url-parenthesis");

    let output = snq()
        .current_dir(&dir)
        .args(["url", "10.1000/snq-example(1)"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "https://sci-net.xyz/10.1000/snq-example%281%29\n"
    );

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn request_all_json_prints_empty_array_for_empty_queue() {
    let dir = temp_workspace("request-empty-json");

    let request = snq()
        .current_dir(&dir)
        .args([
            "request",
            "--all",
            "--reward",
            "1",
            "--budget-check",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(request.status.success());
    assert_eq!(String::from_utf8_lossy(&request.stdout), "[]\n");
    assert!(request.stderr.is_empty());

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn browsers_json_reports_env_override() {
    let dir = temp_workspace("browsers-json");
    let browser = dir.join("missing-browser");

    let output = snq()
        .current_dir(&dir)
        .env("SCINET_QUEUE_BROWSER", &browser)
        .args(["browsers", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["override_env"], "SCINET_QUEUE_BROWSER");
    assert_eq!(value["preference_path"], ".snq/browser.json");
    assert!(value["selected"].is_null());
    assert_eq!(value["browsers"][0]["source"], "env");
    assert_eq!(value["browsers"][0]["available"], false);

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn browsers_set_saves_workspace_preference() {
    let dir = temp_workspace("browsers-set");
    let browser = dir.join("fake-firefox");
    write_fake_browser(&browser);

    let set = snq()
        .current_dir(&dir)
        .args(["browsers", "--set", browser.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(set.status.success());
    assert!(set.stderr.is_empty());

    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join(".snq/browser.json")).unwrap()).unwrap();
    assert_eq!(value["engine"], "firefox");
    assert_eq!(value["path"], browser.to_str().unwrap());

    let output = snq()
        .current_dir(&dir)
        .args(["browsers", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["selected"]["source"], "preference");
    assert_eq!(value["selected"]["path"], browser.to_str().unwrap());

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn browsers_clear_removes_workspace_preference() {
    let dir = temp_workspace("browsers-clear");
    fs::create_dir_all(dir.join(".snq")).unwrap();
    fs::write(
        dir.join(".snq/browser.json"),
        "{\"engine\":\"chromium\",\"path\":\"/missing\"}\n",
    )
    .unwrap();

    let output = snq()
        .current_dir(&dir)
        .args(["browsers", "--clear"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "browser preference cleared\n"
    );
    assert!(!dir.join(".snq/browser.json").exists());

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_browser_preference_errors_before_launch() {
    let dir = temp_workspace("missing-browser-preference");
    let browser = dir.join("deleted-firefox");
    fs::create_dir_all(dir.join(".snq")).unwrap();
    let preference = serde_json::json!({
        "engine": "firefox",
        "path": browser,
    });
    fs::write(
        dir.join(".snq/browser.json"),
        format!("{}\n", serde_json::to_string(&preference).unwrap()),
    )
    .unwrap();

    let output = snq()
        .current_dir(&dir)
        .args(["session", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());

    let value: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(
        value["error"]
            .as_str()
            .unwrap()
            .contains("configured browser does not exist")
    );

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invalid_browser_preference_is_reported_and_recoverable() {
    let dir = temp_workspace("invalid-browser-preference");
    fs::create_dir_all(dir.join(".snq")).unwrap();
    fs::write(dir.join(".snq/browser.json"), "{not json\n").unwrap();

    let list = snq()
        .current_dir(&dir)
        .args(["browsers", "--json"])
        .output()
        .unwrap();

    assert!(list.status.success());
    assert!(list.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    let error = value["preference_error"].as_str().unwrap();
    assert!(error.contains("could not parse browser preference .snq/browser.json"));
    assert!(error.contains("snq browsers --clear"));

    let session = snq()
        .current_dir(&dir)
        .args(["session", "--json"])
        .output()
        .unwrap();

    assert!(!session.status.success());
    assert!(session.stdout.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&session.stderr).unwrap();
    let error = value["error"].as_str().unwrap();
    assert!(error.contains("could not parse browser preference .snq/browser.json"));
    assert!(error.contains("snq browsers --clear"));

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn fetch_json_prints_empty_array_for_empty_queue() {
    let dir = temp_workspace("fetch-empty-json");

    let fetch = snq()
        .current_dir(&dir)
        .args(["fetch", "--json"])
        .output()
        .unwrap();

    assert!(fetch.status.success());
    assert_eq!(String::from_utf8_lossy(&fetch.stdout), "[]\n");
    assert!(fetch.stderr.is_empty());

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn watch_json_skips_inactive_entries_without_browser() {
    let dir = temp_workspace("watch-inactive-json");

    let add = snq()
        .current_dir(&dir)
        .args(["add", "10.1000/snq-example"])
        .output()
        .unwrap();
    assert!(add.status.success());

    let approve = snq()
        .current_dir(&dir)
        .args(["approve", "10.1000/snq-example", "--force"])
        .output()
        .unwrap();
    assert!(approve.status.success());

    let watch = snq()
        .current_dir(&dir)
        .env("SCINET_QUEUE_BROWSER", dir.join("missing-browser"))
        .args(["watch", "--json"])
        .output()
        .unwrap();

    assert!(watch.status.success());
    assert_eq!(String::from_utf8_lossy(&watch.stdout), "[]\n");
    assert!(watch.stderr.is_empty());

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn approve_json_marks_entry_approved() {
    let dir = temp_workspace("approve-json");

    let add = snq()
        .current_dir(&dir)
        .args(["add", "10.1000/snq-example"])
        .output()
        .unwrap();
    assert!(add.status.success());

    let approve = snq()
        .current_dir(&dir)
        .args(["approve", "10.1000/snq-example", "--force", "--json"])
        .output()
        .unwrap();

    assert!(approve.status.success());
    assert!(approve.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&approve.stdout).unwrap();
    assert_eq!(value["doi"], "10.1000/snq-example");
    assert_eq!(value["status"], "approved");
    assert_eq!(value["forced"], true);

    let list = snq()
        .current_dir(&dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    assert!(list.status.success());
    let entries: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(entries[0]["status"], "approved");

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn approve_json_errors_are_machine_readable() {
    let dir = temp_workspace("approve-json-error");

    let add = snq()
        .current_dir(&dir)
        .args(["add", "10.1000/snq-example"])
        .output()
        .unwrap();
    assert!(add.status.success());

    let approve = snq()
        .current_dir(&dir)
        .args(["approve", "10.1000/snq-example", "--json"])
        .output()
        .unwrap();

    assert!(!approve.status.success());
    assert!(approve.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&approve.stderr).contains("snq:"));
    let value: serde_json::Value = serde_json::from_slice(&approve.stderr).unwrap();
    assert!(
        value["error"]
            .as_str()
            .unwrap()
            .contains("is queued, not fetched")
    );

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn json_errors_are_machine_readable() {
    let home = temp_workspace("json-error-home");
    let dir = temp_workspace("json-error-workspace");

    let session = snq()
        .current_dir(&dir)
        .env("HOME", &home)
        .env("SCINET_QUEUE_BROWSER", home.join("missing-browser"))
        .args(["session", "--json"])
        .output()
        .unwrap();

    assert!(!session.status.success());
    assert!(session.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&session.stderr).contains("snq:"));

    let value: serde_json::Value = serde_json::from_slice(&session.stderr).unwrap();
    assert!(
        value["error"]
            .as_str()
            .unwrap()
            .contains("SCINET_QUEUE_BROWSER does not exist")
    );

    fs::remove_dir_all(home).unwrap();
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn parser_json_errors_are_machine_readable() {
    let dir = temp_workspace("json-parse-errors");

    let cases = [
        vec!["request", "--all", "--reward", "0", "--json"],
        vec!["fetch", "--poll", "0", "--json"],
        vec!["view", "--bad", "--json"],
    ];

    for args in cases {
        let output = snq().current_dir(&dir).args(args).output().unwrap();

        assert!(!output.status.success());
        assert!(output.stdout.is_empty());

        let value: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert!(
            value["error"]
                .as_str()
                .is_some_and(|error| !error.is_empty())
        );
    }

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn doctor_json_reports_failures_with_nonzero_exit() {
    let home = temp_workspace("doctor-json-home");
    let dir = temp_workspace("doctor-json-workspace");

    let doctor = snq()
        .current_dir(&dir)
        .env("HOME", &home)
        .env("SCINET_QUEUE_BROWSER", home.join("missing-browser"))
        .args(["doctor", "--json"])
        .output()
        .unwrap();

    assert!(!doctor.status.success());
    assert!(String::from_utf8_lossy(&doctor.stderr).contains("doctor: checks failed"));

    let value: serde_json::Value = serde_json::from_slice(&doctor.stdout).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["browser"]["ok"], false);
    assert_eq!(value["queue"]["ok"], true);

    fs::remove_dir_all(home).unwrap();
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn doctor_json_redacts_home_paths() {
    let home = temp_workspace("doctor-redact-home");
    let dir = temp_workspace("doctor-redact-workspace");
    let browser = home.join("missing-browser");

    let doctor = snq()
        .current_dir(&dir)
        .env("HOME", &home)
        .env("SCINET_QUEUE_BROWSER", &browser)
        .args(["doctor", "--json", "--redact"])
        .output()
        .unwrap();

    assert!(!doctor.status.success());

    let value: serde_json::Value = serde_json::from_slice(&doctor.stdout).unwrap();
    let message = value["browser"]["message"].as_str().unwrap();
    assert!(message.contains('~'));
    assert!(message.contains("missing-browser"));
    assert!(!message.contains(home.to_str().unwrap()));

    fs::remove_dir_all(home).unwrap();
    fs::remove_dir_all(dir).unwrap();
}

fn unix_time() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
}
