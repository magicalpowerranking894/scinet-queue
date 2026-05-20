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
    format!("{}:{}:{counter}", std::process::id(), unix_time_millis())
}

pub(crate) fn owner_lock_file_can_be_reclaimed(path: &Path, stale_after: Duration) -> bool {
    if let Ok(contents) = fs::read_to_string(path) {
        if let Some(pid) = parse_lock_owner_pid(contents.trim()) {
            return !process_is_running(pid);
        }
    }

    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .map(|age| age > stale_after)
        .unwrap_or(false)
}

fn parse_lock_owner_pid(token: &str) -> Option<u32> {
    let (pid, _) = token.split_once(':')?;
    pid.parse().ok()
}

fn unix_time_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
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

#[cfg(not(any(unix, windows)))]
fn process_is_running(pid: u32) -> bool {
    pid == std::process::id()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_tokens_include_current_process_owner() {
        let token = lock_token();

        assert_eq!(parse_lock_owner_pid(&token), Some(std::process::id()));
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
    fn implausible_process_lock_is_reclaimed() {
        let dir =
            std::env::temp_dir().join(format!("snq-dead-owner-lock-test-{}", std::process::id()));
        let path = dir.join("lock");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, format!("{}:{}:0\n", u32::MAX, unix_time_millis())).unwrap();

        assert!(owner_lock_file_can_be_reclaimed(
            &path,
            Duration::from_secs(60 * 60)
        ));

        let _ = fs::remove_dir_all(dir);
    }
}
