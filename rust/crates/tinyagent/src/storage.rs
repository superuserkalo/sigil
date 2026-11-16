//! Durable sessions: the journalled record shapes, the [`Storage`] contract and
//! an in-memory store that lives and dies with the object.
//!
//! A session is an ordered list of records. A store may serialize them however
//! it likes; every record carries its `sequence`, which is `1..n` with no gaps.
//! An append is fenced: it names the count the writer expects, and a store that
//! holds a different count writes nothing and answers [`Error::Fenced`].

use crate::types::{Error, Message, Result, RunFailure, StoredOptions, Usage, JOURNAL_VERSION};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Mutex};

/// One journaled record: its position in the session, the format it was written
/// in and the body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Record {
    pub sequence: u64,
    #[serde(default = "journal_version")]
    pub version: u32,
    #[serde(flatten)]
    pub body: RecordBody,
}

fn journal_version() -> u32 {
    JOURNAL_VERSION
}

impl Record {
    pub fn new(sequence: u64, body: RecordBody) -> Self {
        Self {
            sequence,
            version: JOURNAL_VERSION,
            body,
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// How a run's `run_failed` record ended it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Failed,
    Aborted,
}

/// Every record a session writes.
///
/// The `type` tag and the field names are the journal contract: a session
/// written by this crate can be reopened by it. Changing one means bumping
/// [`JOURNAL_VERSION`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RecordBody {
    /// The first record: which session this is and the configuration that created it.
    Session {
        session_id: String,
        configuration: String,
    },
    /// The agent configuration changed. The journal says when, so a continuation is explainable.
    Configuration {
        configuration: String,
        model: String,
        instructions: String,
        tools: Vec<String>,
    },
    /// A run started. The prompt is part of this record, so an open run always has one.
    RunStart {
        run_id: String,
        item: Message,
        #[serde(default, skip_serializing_if = "StoredOptions::is_empty")]
        options: StoredOptions,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
    },
    /// One history item of the open run.
    Item { item: Message },
    /// The intent to run one tool call, written before it runs. The sequence is
    /// the call's execution ID, so a rerun is recognisably the same call.
    ToolIntent {
        call_id: String,
        tool_name: String,
        arguments: Value,
        idempotent: bool,
    },
    /// An interrupted call is about to run again.
    ToolRerun { call_id: String, tool_name: String },
    /// A paused call is about to be answered.
    ToolResumed { call_id: String, tool_name: String },
    /// The call paused the run. It has its end and waits for a result.
    ToolPaused {
        call_id: String,
        tool_name: String,
        payload: Value,
    },
    /// A settled tool call as the model will see it. `value` is the typed output a
    /// `final_output` call delivered.
    ToolResult {
        item: Message,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Value>,
        #[serde(default, skip_serializing_if = "is_false")]
        interrupted: bool,
    },
    /// The run stopped at a pause and can be resumed.
    RunPaused { run_id: String, usage: Usage },
    /// The run finished. Its items join the transcript.
    RunCommit {
        run_id: String,
        text: String,
        usage: Usage,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Value>,
    },
    /// The run was abandoned, failed or cancelled. Its items do not join the transcript.
    RunFailed {
        run_id: String,
        status: RunStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<RunFailure>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
    },
    /// A summary of earlier history, with the digest of what it covered.
    Checkpoint {
        covered: usize,
        summary: String,
        history_hash: String,
    },
    /// A human settled an interrupted non-idempotent call without continuing the run.
    Reconciled { note: String },
}

/// Where sessions are kept. Every operation is fenced or ordered by `sequence`,
/// and the store is the truth: a session object only ever mirrors what it wrote.
pub trait Storage: Send + Sync {
    /// The session's records in order, or `None` when there is no such session.
    fn load<'a>(&'a self, session_id: &'a str) -> BoxFuture<'a, Result<Option<Vec<Record>>>>;

    /// Store `body` as record `expected + 1` and return that sequence. The record
    /// must be durable before this returns. When the session does not hold exactly
    /// `expected` records, write nothing and return [`Error::Fenced`]. `expected`
    /// `0` creates the session. Any other error means the outcome is unknown.
    fn append<'a>(
        &'a self,
        session_id: &'a str,
        expected: u64,
        body: RecordBody,
    ) -> BoxFuture<'a, Result<u64>>;

    /// Every stored session ID.
    fn list(&self) -> BoxFuture<'_, Result<Vec<String>>>;

    /// Let go of the session's resources. Never deletes data.
    fn release<'a>(&'a self, session_id: &'a str) -> BoxFuture<'a, Result<()>>;
}

/// Sessions that live and die with this object. It can be shared by several sessions.
#[derive(Debug, Default)]
pub struct MemoryStorage {
    sessions: Mutex<BTreeMap<String, Vec<Record>>>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Storage for MemoryStorage {
    fn load<'a>(&'a self, session_id: &'a str) -> BoxFuture<'a, Result<Option<Vec<Record>>>> {
        let records = self
            .sessions
            .lock()
            .map_err(|_| Error::Storage("memory storage lock was poisoned".into()))
            .map(|sessions| sessions.get(session_id).cloned());
        Box::pin(std::future::ready(records))
    }

    fn append<'a>(
        &'a self,
        session_id: &'a str,
        expected: u64,
        body: RecordBody,
    ) -> BoxFuture<'a, Result<u64>> {
        let result = (|| {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| Error::Storage("memory storage lock was poisoned".into()))?;
            let records = sessions.entry(session_id.to_string()).or_default();
            if records.len() as u64 != expected {
                return Err(fenced());
            }
            let sequence = expected + 1;
            records.push(Record::new(sequence, body));
            Ok(sequence)
        })();
        Box::pin(std::future::ready(result))
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<String>>> {
        let listed = self
            .sessions
            .lock()
            .map_err(|_| Error::Storage("memory storage lock was poisoned".into()))
            .map(|sessions| sessions.keys().cloned().collect());
        Box::pin(std::future::ready(listed))
    }

    fn release<'a>(&'a self, _session_id: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(std::future::ready(Ok(())))
    }
}

/// Another writer got there first: the append wrote nothing.
pub fn fenced() -> Error {
    Error::Fenced("another writer has appended to this session; reopen it".into())
}

/// The storage contract as a check any implementation can run.
///
/// It leaves two small sessions behind, because a store never deletes. The
/// first rule the store breaks comes back as [`Error::Storage`] naming it.
pub async fn check_storage(storage: &dyn Storage) -> Result<()> {
    let rule = |message: String| Error::Storage(format!("storage contract: {message}"));
    let unique = crate::types::random_id();
    let first = format!("s_check_{unique}_a");
    let second = format!("s_check_{unique}_b");
    let header = || RecordBody::Session {
        session_id: "check".into(),
        configuration: "created by check_storage".into(),
    };
    let check = |holds: bool, message: &str| -> Result<()> {
        if holds {
            Ok(())
        } else {
            Err(rule(message.into()))
        }
    };
    macro_rules! fenced {
        ($expected:expr, $message:expr) => {
            match storage
                .append(
                    &first,
                    $expected,
                    RecordBody::Item {
                        item: Message::user("stray"),
                    },
                )
                .await
            {
                Err(Error::Fenced(_)) => {}
                Err(other) => {
                    return Err(rule(format!("{}: {other}", $message)));
                }
                Ok(_) => return Err(rule($message.into())),
            }
        };
    }

    check(
        storage.load(&first).await?.is_none(),
        "an unknown session is not found",
    )?;
    fenced!(1, "appending to a session that does not exist is fenced");
    check(
        storage.append(&first, 0, header()).await? == 1,
        "the first record has sequence 1",
    )?;
    fenced!(0, "creating a session twice is fenced");
    let record = RecordBody::Item {
        item: Message::user("naive snowman line\nsecond line"),
    };
    check(
        storage.append(&first, 1, record.clone()).await? == 2,
        "append returns expected + 1",
    )?;
    fenced!(1, "an append behind the stored count is fenced");
    fenced!(3, "an append ahead of the stored count is fenced");
    check(
        storage
            .append(&first, 2, RecordBody::Reconciled { note: "end".into() })
            .await?
            == 3,
        "a fenced append wrote nothing",
    )?;
    let stored = storage.load(&first).await?.unwrap_or_default();
    check(
        stored.len() == 3
            && stored.iter().enumerate().all(|(index, record)| {
                record.sequence == index as u64 + 1 && record.version == JOURNAL_VERSION
            }),
        "load returns every record, in order, with gapless sequences",
    )?;
    check(
        stored[1].body == record,
        "load returns the records that were appended",
    )?;

    check(
        storage.append(&second, 0, header()).await? == 1,
        "sessions are independent",
    )?;
    let listed = storage.list().await?;
    check(
        listed.contains(&first) && listed.contains(&second),
        "list names every stored session",
    )?;
    storage.release(&first).await?;
    check(
        storage.load(&first).await? == Some(stored.clone()),
        "release keeps the data",
    )?;
    check(
        storage
            .append(
                &first,
                3,
                RecordBody::Reconciled {
                    note: "later".into(),
                },
            )
            .await?
            == 4,
        "a released session can be appended to again",
    )?;
    storage.release(&first).await?;
    storage.release(&second).await?;
    check(
        storage.load(&first).await?.map(|records| records.len()) == Some(4),
        "a session holds every record appended to it",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_storage_satisfies_the_contract() {
        check_storage(&MemoryStorage::new()).await.unwrap();
    }

    #[tokio::test]
    async fn records_round_trip_through_json() {
        let record = Record::new(
            3,
            RecordBody::ToolResult {
                item: Message::tool_result("call_1", "bash", "ok", false),
                value: Some(serde_json::json!({"answer": 42})),
                interrupted: false,
            },
        );
        let text = serde_json::to_string(&record).unwrap();
        let parsed: Record = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, record);
        assert_eq!(parsed.sequence, 3);
    }
}
