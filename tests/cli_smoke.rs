use std::fs;
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

#[test]
fn help_and_version_work() {
    let help = snq().arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Usage:"));

    let version = snq().arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).starts_with("snq "));
}

#[test]
fn queue_round_trip_works_from_cli() {
    let dir = temp_workspace("queue-round-trip");

    let add = snq()
        .current_dir(&dir)
        .args(["add", "10.1287/mnsc.2024.05040"])
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
    assert!(list.contains("\"doi\": \"10.1287/mnsc.2024.05040\""));
    assert!(list.contains("\"status\": \"queued\""));

    let remove = snq()
        .current_dir(&dir)
        .args(["remove", "10.1287/mnsc.2024.05040"])
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
    fs::write(&import, "10.1287/mnsc.2024.05040\n").unwrap();

    let import_result = snq()
        .current_dir(&dir)
        .args(["import", import.to_str().unwrap(), "extra"])
        .output()
        .unwrap();
    assert!(!import_result.status.success());
    assert!(String::from_utf8_lossy(&import_result.stderr).contains("unexpected argument"));

    let remove_result = snq()
        .current_dir(&dir)
        .args(["remove", "10.1287/mnsc.2024.05040", "extra"])
        .output()
        .unwrap();
    assert!(!remove_result.status.success());
    assert!(String::from_utf8_lossy(&remove_result.stderr).contains("unexpected argument"));

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn request_all_json_prints_empty_array_for_empty_queue() {
    let dir = temp_workspace("request-empty-json");

    let request = snq()
        .current_dir(&dir)
        .args(["request", "--all", "--reward", "1", "--json"])
        .output()
        .unwrap();

    assert!(request.status.success());
    assert_eq!(String::from_utf8_lossy(&request.stdout), "[]\n");
    assert!(request.stderr.is_empty());

    fs::remove_dir_all(dir).unwrap();
}

fn unix_time() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
}
