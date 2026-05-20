use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

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

#[cfg(test)]
#[derive(Debug, Eq, PartialEq)]
struct LockToken<'a> {
    pid: u32,
    owner_name: Option<&'a str>,
}

#[cfg(test)]
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
        .map(normalize_process_name)
        .unwrap_or_else(|| "snq".to_string())
}

fn path_file_name(path: &Path) -> Option<&str> {
    path.file_name().and_then(|name| name.to_str())
}

fn normalize_process_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
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
    fn process_name_normalization_is_case_insensitive() {
        assert_eq!(normalize_process_name("snq"), "snq");
        assert_eq!(normalize_process_name("snq.exe"), "snq");
        assert_eq!(normalize_process_name("SNQ.EXE"), "snq");
        assert_eq!(normalize_process_name("SnQ.ExE"), "snq");
    }
}
