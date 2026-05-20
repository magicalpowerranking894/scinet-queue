use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct QueueEntry {
    pub doi: String,
    pub status: QueueStatus,
    pub created_at: u64,
    pub updated_at: u64,
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
pub enum QueueStatus {
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
pub struct Queue {
    path: PathBuf,
}

impl Queue {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn add(&self, raw_doi: &str) -> Result<AddResult, QueueError> {
        let doi = normalize_doi(raw_doi)?;
        let mut entries = self.read()?;

        if entries.iter().any(|entry| entry.doi == doi) {
            return Ok(AddResult::AlreadyQueued(doi));
        }

        entries.push(QueueEntry::new(doi.clone(), unix_time()));
        self.write(&entries)?;

        Ok(AddResult::Queued(doi))
    }

    pub fn list(&self) -> Result<Vec<QueueEntry>, QueueError> {
        self.read()
    }

    pub fn remove(&self, raw_doi: &str) -> Result<RemoveResult, QueueError> {
        let doi = normalize_doi(raw_doi)?;
        let mut entries = self.read()?;
        let before = entries.len();

        entries.retain(|entry| entry.doi != doi);

        if entries.len() == before {
            return Ok(RemoveResult::NotFound(doi));
        }

        self.write(&entries)?;
        Ok(RemoveResult::Removed(doi))
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
pub enum AddResult {
    Queued(String),
    AlreadyQueued(String),
}

#[derive(Debug, Eq, PartialEq)]
pub enum RemoveResult {
    Removed(String),
    NotFound(String),
}

#[derive(Debug)]
pub enum QueueError {
    Io(io::Error),
    Json(serde_json::Error),
    CorruptLine {
        path: PathBuf,
        line: usize,
        source: serde_json::Error,
    },
    InvalidDoi(String),
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

pub fn default_queue_path() -> PathBuf {
    PathBuf::from(".snq").join("queue.jsonl")
}

pub fn normalize_doi(raw: &str) -> Result<String, QueueError> {
    let doi = raw
        .trim()
        .trim_start_matches("doi:")
        .trim_start_matches("DOI:")
        .trim_start_matches("https://doi.org/")
        .trim_start_matches("http://doi.org/")
        .trim()
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
}
