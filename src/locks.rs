use std::fs;
use std::path::Path;
use std::process::Command;
#[cfg(unix)]
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static LOCK_TOKEN_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn lock_token() -> String {
    let counter = LOCK_TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "{}:{}:{counter}:{}",
        std::process::id(),
        unix_time_millis(),
        current_process_name()
    )
}

pub(crate) fn owner_lock_file_can_be_reclaimed(path: &Path, stale_after: Duration) -> bool {
    if let Ok(contents) = fs::read_to_string(path) {
        if let Some(token) = parse_lock_token(contents.trim()) {
            if !process_is_running(token.pid) {
                return true;
            }

            if let (Some(lock_owner), Some(process_owner)) =
                (token.owner_name, process_name(token.pid))
            {
                return lock_owner != process_owner;
            }

            return false;
        }
    }

    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .map(|age| age > stale_after)
        .unwrap_or(false)
}

#[derive(Debug, Eq, PartialEq)]
struct LockToken<'a> {
    pid: u32,
    owner_name: Option<&'a str>,
}

fn parse_lock_token(token: &str) -> Option<LockToken<'_>> {
    let mut parts = token.split(':');
    let pid = parts.next()?.parse().ok()?;

    if pid == 0 {
        return None;
    }

    let _millis = parts.next()?;
    let _counter = parts.next();
    let owner_name = parts.next().filter(|value| !value.is_empty());

    Some(LockToken { pid, owner_name })
}

fn unix_time_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn current_process_name() -> String {
    std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(path_file_name)
        .unwrap_or("snq")
        .to_ascii_lowercase()
}

fn path_file_name(path: &Path) -> Option<&str> {
    path.file_name().and_then(|name| name.to_str())
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    if pid > i32::MAX as u32 {
        return false;
    }

    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn process_name(pid: u32) -> Option<String> {
    fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .as_deref()
        .and_then(path_file_name)
        .map(|name| name.to_ascii_lowercase())
}

#[cfg(all(unix, not(target_os = "linux")))]
fn process_name(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    path_file_name(Path::new(&path)).map(|name| name.to_ascii_lowercase())
}

#[cfg(windows)]
fn process_is_running(pid: u32) -> bool {
    let Ok(output) = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
    else {
        return false;
    };

    if !output.status.success() {
        return false;
    }

    String::from_utf8_lossy(&output.stdout).contains(&format!(",\"{pid}\","))
}

#[cfg(windows)]
fn process_name(pid: u32) -> Option<String> {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let output = String::from_utf8_lossy(&output.stdout);
    let first = output.lines().next()?.split(',').next()?.trim_matches('"');

    (!first.is_empty() && !first.eq_ignore_ascii_case("INFO")).then(|| {
        first
            .strip_suffix(".exe")
            .unwrap_or(first)
            .to_ascii_lowercase()
    })
}

#[cfg(not(any(unix, windows)))]
fn process_is_running(pid: u32) -> bool {
    pid == std::process::id()
}

#[cfg(not(any(unix, windows)))]
fn process_name(pid: u32) -> Option<String> {
    (pid == std::process::id()).then(current_process_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_tokens_include_current_process_owner() {
        let token = lock_token();
        let owner_name = current_process_name();

        assert_eq!(
            parse_lock_token(&token),
            Some(LockToken {
                pid: std::process::id(),
                owner_name: Some(owner_name.as_str())
            })
        );
    }

    #[test]
    fn current_process_lock_is_not_reclaimed_even_when_old() {
        let dir = std::env::temp_dir().join(format!("snq-owner-lock-test-{}", std::process::id()));
        let path = dir.join("lock");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, format!("{}:1:0\n", std::process::id())).unwrap();

        assert!(!owner_lock_file_can_be_reclaimed(
            &path,
            Duration::from_millis(1)
        ));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn reused_pid_with_different_owner_is_reclaimed() {
        let dir = std::env::temp_dir().join(format!("snq-reused-pid-test-{}", std::process::id()));
        let path = dir.join("lock");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &path,
            format!("{}:1:0:not-current-process\n", std::process::id()),
        )
        .unwrap();

        assert!(owner_lock_file_can_be_reclaimed(
            &path,
            Duration::from_secs(60 * 60)
        ));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn implausible_process_lock_is_reclaimed() {
        let dir =
            std::env::temp_dir().join(format!("snq-dead-owner-lock-test-{}", std::process::id()));
        let path = dir.join("lock");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &path,
            format!("{}:{}:0\n", 99_999_999_u32, unix_time_millis()),
        )
        .unwrap();

        assert!(owner_lock_file_can_be_reclaimed(
            &path,
            Duration::from_secs(60 * 60)
        ));

        let _ = fs::remove_dir_all(dir);
    }
}
