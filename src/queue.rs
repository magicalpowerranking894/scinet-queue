use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const QUEUE_LOCK_TIMEOUT: Duration = Duration::from_secs(60);
const QUEUE_LOCK_POLL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct QueueEntry {
    pub(crate) doi: String,
    pub(crate) status: QueueStatus,
    pub(crate) created_at: u64,
    pub(crate) updated_at: u64,
}

impl QueueEntry {
    fn new(doi: String, now: u64) -> Self {
        Self {
            doi,
            status: QueueStatus::Queued,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum QueueStatus {
    Queued,
    Requested,
    Working,
    Fetched,
    Approved,
}

impl fmt::Display for QueueStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = match self {
            QueueStatus::Queued => "queued",
            QueueStatus::Requested => "requested",
            QueueStatus::Working => "working",
            QueueStatus::Fetched => "fetched",
            QueueStatus::Approved => "approved",
        };

        f.write_str(status)
    }
}

#[derive(Debug)]
pub(crate) struct Queue {
    path: PathBuf,
}

impl Queue {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub(crate) fn add(&self, raw_doi: &str) -> Result<AddResult, QueueError> {
        let doi = normalize_doi(raw_doi)?;
        let _lock = self.lock()?;
        let mut entries = self.read()?;

        if entries.iter().any(|entry| entry.doi == doi) {
            return Ok(AddResult::AlreadyQueued(doi));
        }

        entries.push(QueueEntry::new(doi.clone(), unix_time()));
        self.write(&entries)?;

        Ok(AddResult::Queued(doi))
    }

    pub(crate) fn list(&self) -> Result<Vec<QueueEntry>, QueueError> {
        self.read()
    }

    pub(crate) fn remove(&self, raw_doi: &str) -> Result<RemoveResult, QueueError> {
        let doi = normalize_doi(raw_doi)?;
        let _lock = self.lock()?;
        let mut entries = self.read()?;
        let before = entries.len();

        entries.retain(|entry| entry.doi != doi);

        if entries.len() == before {
            return Ok(RemoveResult::NotFound(doi));
        }

        self.write(&entries)?;
        Ok(RemoveResult::Removed(doi))
    }

    pub(crate) fn set_status(
        &self,
        raw_doi: &str,
        status: QueueStatus,
    ) -> Result<StatusResult, QueueError> {
        let doi = normalize_doi(raw_doi)?;
        let _lock = self.lock()?;
        let mut entries = self.read()?;

        if let Some(entry) = entries.iter_mut().find(|entry| entry.doi == doi) {
            entry.status = status;
            entry.updated_at = unix_time();
            self.write(&entries)?;

            return Ok(StatusResult::Updated(doi));
        }

        Ok(StatusResult::NotFound(doi))
    }

    fn lock(&self) -> Result<QueueLock, QueueError> {
        QueueLock::acquire(&self.lock_path(), QUEUE_LOCK_TIMEOUT)
    }

    fn lock_path(&self) -> PathBuf {
        self.path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("queue.lock")
    }

    fn read(&self) -> Result<Vec<QueueEntry>, QueueError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&self.path)?;
        let mut entries = Vec::new();

        for (index, line) in io::BufReader::new(file).lines().enumerate() {
            let line = line?;

            if line.trim().is_empty() {
                continue;
            }

            let entry = serde_json::from_str(&line).map_err(|source| QueueError::CorruptLine {
                path: self.path.clone(),
                line: index + 1,
                source,
            })?;

            entries.push(entry);
        }

        Ok(entries)
    }

    fn write(&self, entries: &[QueueEntry]) -> Result<(), QueueError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let temp_path = temp_path_for(&self.path);
        let mut file = fs::File::create(&temp_path)?;

        for entry in entries {
            serde_json::to_writer(&mut file, entry)?;
            file.write_all(b"\n")?;
        }

        file.sync_all()?;
        fs::rename(&temp_path, &self.path)?;

        Ok(())
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum AddResult {
    Queued(String),
    AlreadyQueued(String),
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum RemoveResult {
    Removed(String),
    NotFound(String),
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum StatusResult {
    Updated(String),
    NotFound(String),
}

#[derive(Debug)]
pub(crate) enum QueueError {
    Io(io::Error),
    Json(serde_json::Error),
    CorruptLine {
        path: PathBuf,
        line: usize,
        source: serde_json::Error,
    },
    InvalidDoi(String),
    QueueLocked(PathBuf),
}

impl fmt::Display for QueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueueError::Io(error) => write!(f, "{error}"),
            QueueError::Json(error) => write!(f, "{error}"),
            QueueError::CorruptLine { path, line, source } => {
                write!(
                    f,
                    "could not parse {} line {}: {}",
                    path.display(),
                    line,
                    source
                )
            }
            QueueError::InvalidDoi(doi) => write!(f, "invalid DOI `{doi}`"),
            QueueError::QueueLocked(path) => write!(
                f,
                "queue is already in use: {}; wait for the other snq command to finish, or remove the lock if no snq process is running",
                path.display()
            ),
        }
    }
}

impl From<io::Error> for QueueError {
    fn from(error: io::Error) -> Self {
        QueueError::Io(error)
    }
}

impl From<serde_json::Error> for QueueError {
    fn from(error: serde_json::Error) -> Self {
        QueueError::Json(error)
    }
}

pub(crate) fn default_queue_path() -> PathBuf {
    PathBuf::from(".snq").join("queue.jsonl")
}

pub(crate) fn normalize_doi(raw: &str) -> Result<String, QueueError> {
    let trimmed = raw.trim().trim_matches(['<', '>']);
    let lower = trimmed.to_ascii_lowercase();
    let doi = if lower.starts_with("doi:") {
        &trimmed[4..]
    } else if lower.starts_with("https://doi.org/") {
        &trimmed[16..]
    } else if lower.starts_with("http://doi.org/") {
        &trimmed[15..]
    } else {
        trimmed
    }
    .trim()
    .split(['?', '#'])
    .next()
    .unwrap_or_default()
    .trim_end_matches(['.', ',', ';', ':', ')', ']', '}', '>'])
    .to_ascii_lowercase();

    if doi.starts_with("10.")
        && doi.contains('/')
        && doi.len() > 7
        && doi.chars().all(|ch| !ch.is_whitespace())
    {
        Ok(doi)
    } else {
        Err(QueueError::InvalidDoi(raw.trim().to_string()))
    }
}

fn temp_path_for(path: &Path) -> PathBuf {
    let pid = std::process::id();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("queue.jsonl");

    path.with_file_name(format!(".{file_name}.{pid}.tmp"))
}

#[derive(Debug)]
struct QueueLock {
    file: File,
}

impl QueueLock {
    fn acquire(path: &Path, timeout: Duration) -> Result<Self, QueueError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let start = Instant::now();
        let token = lock_token();
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        loop {
            match file.try_lock_exclusive() {
                Ok(()) => {
                    file.set_len(0)?;
                    writeln!(file, "{token}")?;

                    return Ok(Self { file });
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if start.elapsed() >= timeout {
                        return Err(QueueError::QueueLocked(path.to_path_buf()));
                    }

                    thread::sleep(QUEUE_LOCK_POLL);
                }
                Err(error) => return Err(QueueError::Io(error)),
            }
        }
    }
}

impl Drop for QueueLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn lock_token() -> String {
    format!("{}:{}", std::process::id(), unix_time_millis())
}

fn unix_time_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before UNIX_EPOCH")
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_common_doi_forms() {
        assert_eq!(
            normalize_doi(" https://doi.org/10.1287/MNSC.2024.05040 ").unwrap(),
            "10.1287/mnsc.2024.05040"
        );
        assert_eq!(
            normalize_doi("doi:10.1093/rfs/hhaa075").unwrap(),
            "10.1093/rfs/hhaa075"
        );
        assert_eq!(
            normalize_doi(" Doi:10.7000/Mixed ").unwrap(),
            "10.7000/mixed"
        );
        assert_eq!(
            normalize_doi("HTTPS://DOI.ORG/10.7000/URL").unwrap(),
            "10.7000/url"
        );
        assert_eq!(
            normalize_doi("https://doi.org/10.1000/ABC?utm_source=x#frag").unwrap(),
            "10.1000/abc"
        );
        assert_eq!(
            normalize_doi("doi:10.1093/rfs/hhaa075.").unwrap(),
            "10.1093/rfs/hhaa075"
        );
        assert_eq!(
            normalize_doi("<https://doi.org/10.1287/MNSC.2024.05040>").unwrap(),
            "10.1287/mnsc.2024.05040"
        );
    }

    #[test]
    fn rejects_invalid_doi() {
        assert!(matches!(
            normalize_doi("not-a-doi"),
            Err(QueueError::InvalidDoi(_))
        ));
        assert!(matches!(
            normalize_doi("10.1287/has whitespace"),
            Err(QueueError::InvalidDoi(_))
        ));
    }

    #[test]
    fn queue_add_list_remove_round_trip() {
        let dir = std::env::temp_dir().join(format!("snq-test-{}", std::process::id()));
        let path = dir.join("queue.jsonl");
        let queue = Queue::new(path);

        let first = queue.add("10.1287/mnsc.2024.05040").unwrap();
        let duplicate = queue
            .add("https://doi.org/10.1287/MNSC.2024.05040")
            .unwrap();
        let entries = queue.list().unwrap();
        let removed = queue.remove("10.1287/mnsc.2024.05040").unwrap();
        let after_remove = queue.list().unwrap();

        assert_eq!(
            first,
            AddResult::Queued("10.1287/mnsc.2024.05040".to_string())
        );
        assert_eq!(
            duplicate,
            AddResult::AlreadyQueued("10.1287/mnsc.2024.05040".to_string())
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, QueueStatus::Queued);
        assert_eq!(
            removed,
            RemoveResult::Removed("10.1287/mnsc.2024.05040".to_string())
        );
        assert!(after_remove.is_empty());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn queue_updates_status() {
        let dir = std::env::temp_dir().join(format!("snq-status-test-{}", std::process::id()));
        let path = dir.join("queue.jsonl");
        let queue = Queue::new(path);

        queue.add("10.1287/mnsc.2024.05040").unwrap();
        let result = queue
            .set_status("10.1287/mnsc.2024.05040", QueueStatus::Requested)
            .unwrap();
        let entries = queue.list().unwrap();

        assert_eq!(
            result,
            StatusResult::Updated("10.1287/mnsc.2024.05040".to_string())
        );
        assert_eq!(entries[0].status, QueueStatus::Requested);
        assert!(entries[0].updated_at >= entries[0].created_at);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn queue_lock_rejects_concurrent_acquire() {
        let dir = std::env::temp_dir().join(format!("snq-lock-test-{}", std::process::id()));
        let path = dir.join("queue.lock");
        let _ = fs::remove_dir_all(&dir);

        let lock = QueueLock::acquire(&path, Duration::from_millis(1)).unwrap();
        let second = QueueLock::acquire(&path, Duration::from_millis(1));

        assert!(matches!(second, Err(QueueError::QueueLocked(_))));

        drop(lock);
        assert!(QueueLock::acquire(&path, Duration::from_millis(1)).is_ok());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn queue_lock_ignores_leftover_lock_file() {
        let dir =
            std::env::temp_dir().join(format!("snq-lock-leftover-test-{}", std::process::id()));
        let path = dir.join("queue.lock");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, "99999999:1\n").unwrap();

        assert!(QueueLock::acquire(&path, Duration::from_millis(1)).is_ok());

        let _ = fs::remove_dir_all(dir);
    }
}
