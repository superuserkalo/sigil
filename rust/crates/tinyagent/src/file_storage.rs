//! One append-only JSONL file per session, each record checksummed, plus a lock
//! file per session so a second writer fails fast.
//!
//! A record is one line: `{"payload": "<record json>", "sha256": "<hex>"}`. The
//! checksum covers the payload string exactly as written, so a torn, truncated
//! or edited record is detected instead of silently misread. A final line
//! without its newline was left by a crash mid-write and is discarded, with the
//! file truncated back to the last complete record.
//!
//! File work here is synchronous: records are small and each append is one write
//! and one fsync. A store on a slow or networked disk should be implemented
//! separately with `spawn_blocking`.

use crate::{
    storage::{fenced, Record, RecordBody, Storage},
    Error, Result, JOURNAL_VERSION, MAX_JSONL_LINE_BYTES,
};
use fs2::FileExt;
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

const MAX_SESSION_ID: usize = 100;

/// `s_` followed by 1 to 100 ASCII letters, digits, `_` or `-`.
pub fn valid_session_id(session_id: &str) -> bool {
    session_id.strip_prefix("s_").is_some_and(|rest| {
        !rest.is_empty()
            && rest.len() <= MAX_SESSION_ID
            && rest
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    })
}

/// The envelope one line holds.
#[derive(Debug, Serialize, Deserialize)]
struct Envelope {
    payload: String,
    sha256: String,
}

/// One open journal file and its writer lock. The lock is native and released
/// by the operating system when the process exits, including a crash; the fenced
/// append is what keeps the journal correct.
struct Journal {
    file: File,
    lock: File,
    offset: u64,
    count: u64,
    failed: bool,
}

impl Journal {
    /// Open the journal for `path`. With `create` false, `None` means no such session.
    fn open(path: &Path, create: bool) -> Result<Option<Self>> {
        if !create {
            match fs::metadata(path) {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(storage_error(path, error)),
            }
        } else if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| storage_error(parent, error))?;
        }
        let lock_path = lock_path(path);
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|error| storage_error(&lock_path, error))?;
        match lock.try_lock_exclusive() {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                return Err(Error::SessionBusy);
            }
            Err(error) => return Err(storage_error(&lock_path, error)),
        }
        let file = match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(create)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if create && error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(Error::Fenced("session already exists".into()));
            }
            Err(error) => return Err(storage_error(path, error)),
        };
        if create {
            // The directory entry itself must be durable, or a crash can lose the new file.
            if let Some(parent) = path.parent() {
                if let Ok(directory) = File::open(parent) {
                    let _ = directory.sync_all();
                }
            }
        }
        Ok(Some(Self {
            file,
            lock,
            offset: 0,
            count: 0,
            failed: false,
        }))
    }

    /// Every complete record. A torn final record is discarded.
    fn read(&mut self) -> Result<Vec<Record>> {
        if self.failed {
            return Err(Error::Storage(
                "journal is closed or failed; reopen the session".into(),
            ));
        }
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|error| storage_error(Path::new("journal"), error))?;
        let mut data = Vec::new();
        self.file
            .read_to_end(&mut data)
            .map_err(|error| storage_error(Path::new("journal"), error))?;
        let end = match data.iter().rposition(|byte| *byte == b'\n') {
            Some(index) => index + 1,
            None => 0,
        };
        let mut records = Vec::new();
        for line in data[..end].split(|byte| *byte == b'\n') {
            if line.is_empty() {
                continue;
            }
            let number = records.len() + 1;
            let envelope: Envelope = serde_json::from_slice(line).map_err(|error| {
                Error::Storage(format!("invalid session record {number}: {error}"))
            })?;
            if crate::types::sha256_hex(&envelope.payload) != envelope.sha256 {
                return Err(Error::Storage(format!(
                    "invalid session record {number}: checksum mismatch"
                )));
            }
            let record: Record = serde_json::from_str(&envelope.payload).map_err(|error| {
                Error::Storage(format!("invalid session record {number}: {error}"))
            })?;
            if record.sequence != number as u64 {
                return Err(Error::Storage(format!(
                    "invalid session record {number}: sequence {} out of order",
                    record.sequence
                )));
            }
            if record.version != JOURNAL_VERSION {
                return Err(Error::Unsupported(format!(
                    "session record {number} is format version {}; this build writes version {JOURNAL_VERSION}",
                    record.version
                )));
            }
            records.push(record);
        }
        if end as u64 != data.len() as u64 {
            self.file
                .set_len(end as u64)
                .map_err(|error| storage_error(Path::new("journal"), error))?;
            self.file
                .sync_all()
                .map_err(|error| storage_error(Path::new("journal"), error))?;
        }
        self.offset = end as u64;
        self.count = records.len() as u64;
        Ok(records)
    }

    fn write(&mut self, expected: u64, body: RecordBody) -> Result<u64> {
        if self.failed {
            return Err(Error::Storage(
                "journal is closed or failed; reopen the session".into(),
            ));
        }
        if self.count != expected {
            return Err(fenced());
        }
        let length = self
            .file
            .metadata()
            .map_err(|error| storage_error(Path::new("journal"), error))?
            .len();
        if length != self.offset {
            return Err(fenced());
        }
        let record = Record::new(expected + 1, body);
        let payload = serde_json::to_string(&record)
            .map_err(|error| Error::Storage(format!("session record is not JSON: {error}")))?;
        let envelope = Envelope {
            sha256: crate::types::sha256_hex(&payload),
            payload,
        };
        let mut line = serde_json::to_vec(&envelope)
            .map_err(|error| Error::Storage(format!("session record is not JSON: {error}")))?;
        line.push(b'\n');
        if line.len() > MAX_JSONL_LINE_BYTES {
            return Err(Error::Storage(format!(
                "session record exceeds {MAX_JSONL_LINE_BYTES} bytes"
            )));
        }
        let written = (|| -> io::Result<()> {
            self.file.seek(SeekFrom::Start(self.offset))?;
            self.file.write_all(&line)?;
            self.file.sync_all()
        })();
        if let Err(error) = written {
            self.failed = true;
            return Err(storage_error(Path::new("journal"), error));
        }
        self.offset += line.len() as u64;
        self.count += 1;
        Ok(expected + 1)
    }
}

impl Drop for Journal {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.lock);
    }
}

fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

fn storage_error(path: &Path, error: io::Error) -> Error {
    Error::Storage(format!("{}: {error}", path.display()))
}

/// One append-only JSONL file per session in `directory`. Constructing it touches nothing.
pub struct FileStorage {
    directory: PathBuf,
    journals: Mutex<BTreeMap<String, Journal>>,
}

impl std::fmt::Debug for FileStorage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FileStorage")
            .field("directory", &self.directory)
            .finish_non_exhaustive()
    }
}

impl FileStorage {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            journals: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    fn path(&self, session_id: &str) -> PathBuf {
        self.directory.join(format!("{session_id}.jsonl"))
    }

    fn journals(&self) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, Journal>>> {
        self.journals
            .lock()
            .map_err(|_| Error::Storage("file storage lock was poisoned".into()))
    }

    fn load_sync(&self, session_id: &str) -> Result<Option<Vec<Record>>> {
        if !valid_session_id(session_id) {
            return Err(Error::Configuration("invalid session ID".into()));
        }
        let mut journals = self.journals()?;
        if !journals.contains_key(session_id) {
            match Journal::open(&self.path(session_id), false)? {
                Some(journal) => {
                    journals.insert(session_id.to_string(), journal);
                }
                None => return Ok(None),
            }
        }
        let result = journals
            .get_mut(session_id)
            .expect("journal was just inserted")
            .read();
        if result.is_err() {
            journals.remove(session_id);
        }
        result.map(Some)
    }

    fn append_sync(&self, session_id: &str, expected: u64, body: RecordBody) -> Result<u64> {
        if !valid_session_id(session_id) {
            return Err(Error::Configuration("invalid session ID".into()));
        }
        let mut journals = self.journals()?;
        if !journals.contains_key(session_id) {
            let path = self.path(session_id);
            match Journal::open(&path, expected == 0)? {
                Some(journal) => {
                    journals.insert(session_id.to_string(), journal);
                }
                None => return Err(fenced()),
            }
            if expected > 0 {
                // A journal that was not created here has records to read before counting.
                let journal = journals
                    .get_mut(session_id)
                    .expect("journal was just inserted");
                if let Err(error) = journal.read() {
                    journals.remove(session_id);
                    return Err(error);
                }
            }
        }
        let result = journals
            .get_mut(session_id)
            .expect("journal exists")
            .write(expected, body);
        if result.is_err() {
            journals.remove(session_id);
        }
        result
    }

    fn list_sync(&self) -> Result<Vec<String>> {
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(storage_error(&self.directory, error)),
        };
        let mut sessions = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| storage_error(&self.directory, error))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(session_id) = name.strip_suffix(".jsonl") else {
                continue;
            };
            if valid_session_id(session_id) {
                sessions.push(session_id.to_string());
            }
        }
        sessions.sort();
        Ok(sessions)
    }
}

impl Storage for FileStorage {
    fn load<'a>(&'a self, session_id: &'a str) -> BoxFuture<'a, Result<Option<Vec<Record>>>> {
        let result = self.load_sync(session_id);
        Box::pin(std::future::ready(result))
    }

    fn append<'a>(
        &'a self,
        session_id: &'a str,
        expected: u64,
        body: RecordBody,
    ) -> BoxFuture<'a, Result<u64>> {
        let result = self.append_sync(session_id, expected, body);
        Box::pin(std::future::ready(result))
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<String>>> {
        let result = self.list_sync();
        Box::pin(std::future::ready(result))
    }

    fn release<'a>(&'a self, session_id: &'a str) -> BoxFuture<'a, Result<()>> {
        let released = self.journals().map(|mut journals| {
            journals.remove(session_id);
        });
        Box::pin(std::future::ready(released))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::check_storage;

    fn temporary_directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "tiny-agent-{name}-{}-{}",
            std::process::id(),
            crate::types::random_id()
        ));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[tokio::test]
    async fn file_storage_satisfies_the_contract() {
        let directory = temporary_directory("contract");
        let storage = FileStorage::new(&directory);
        check_storage(&storage).await.unwrap();
        assert!(storage.list().await.unwrap().len() >= 2);
        fs::remove_dir_all(&directory).unwrap();
    }

    #[tokio::test]
    async fn a_torn_final_record_is_discarded_and_reopening_keeps_the_rest() {
        let directory = temporary_directory("torn");
        let storage = FileStorage::new(&directory);
        storage
            .append(
                "s_torn",
                0,
                RecordBody::Session {
                    session_id: "s_torn".into(),
                    configuration: "digest".into(),
                },
            )
            .await
            .unwrap();
        storage
            .append(
                "s_torn",
                1,
                RecordBody::Item {
                    item: crate::Message::user("hi"),
                },
            )
            .await
            .unwrap();
        storage.release("s_torn").await.unwrap();
        let path = directory.join("s_torn.jsonl");
        let complete = fs::metadata(&path).unwrap().len();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"{\"payload\":\"{\\\"sequence\\\":")
            .unwrap();
        file.sync_all().unwrap();
        drop(file);

        let reopened = FileStorage::new(&directory);
        let records = reopened.load("s_torn").await.unwrap().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(fs::metadata(&path).unwrap().len(), complete);
        assert_eq!(
            reopened
                .append(
                    "s_torn",
                    2,
                    RecordBody::Reconciled {
                        note: "after torn tail".into()
                    }
                )
                .await
                .unwrap(),
            3
        );
        fs::remove_dir_all(&directory).unwrap();
    }

    #[tokio::test]
    async fn a_second_open_writer_is_refused() {
        let directory = temporary_directory("locked");
        let first = FileStorage::new(&directory);
        first
            .append(
                "s_lock",
                0,
                RecordBody::Session {
                    session_id: "s_lock".into(),
                    configuration: "digest".into(),
                },
            )
            .await
            .unwrap();
        let second = FileStorage::new(&directory);
        assert!(matches!(
            second.load("s_lock").await,
            Err(Error::SessionBusy)
        ));
        first.release("s_lock").await.unwrap();
        assert_eq!(second.load("s_lock").await.unwrap().unwrap().len(), 1);
        fs::remove_dir_all(&directory).unwrap();
    }
}
