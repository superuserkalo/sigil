//! Match data sharing, the app side: when the user has turned it on, scan Steam's download
//! cache now and then, send the match salts deadlock-api does not have from us yet, and keep a
//! log of every batch in `<data>/match-sharing.json` so the user can see exactly what left.
//!
//! The scan and the request live in `deadlock_core::salts`. Off by default; turning it off
//! stops the next run, and "Forget" deletes the log and the list of what was sent.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use deadlock_core::salts::{self, MatchSalt};
use serde_json::{Value, json};

use crate::app::State;
use crate::jobs::Job;

pub const FILE: &str = "match-sharing.json";
/// Batches kept in the visible log.
const LOG_KEEP: usize = 50;
/// Salts per request.
const BATCH: usize = 100;

#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    /// Unix seconds.
    pub at: u64,
    pub matches: Vec<(u64, String)>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    pub sent: BTreeSet<String>,
    pub log: Vec<Batch>,
}

impl Record {
    pub fn from_json(text: &str) -> Record {
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return Record::default();
        };
        let sent = value["sent"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let log = value["log"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|b| Batch {
                        at: b["at"].as_u64().unwrap_or(0),
                        matches: b["matches"]
                            .as_array()
                            .map(|m| {
                                m.iter()
                                    .filter_map(|e| {
                                        Some((e["match_id"].as_u64()?, e["kind"].as_str()?.to_string()))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        error: b["error"].as_str().map(String::from),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Record { sent, log }
    }

    pub fn to_json(&self) -> String {
        let log: Vec<Value> = self
            .log
            .iter()
            .map(|b| {
                json!({
                    "at": b.at,
                    "matches": b.matches.iter().map(|(id, kind)| json!({"match_id": id, "kind": kind})).collect::<Vec<_>>(),
                    "error": b.error,
                })
            })
            .collect();
        serde_json::to_string_pretty(&json!({ "schema": 1, "sent": self.sent, "log": log }))
            .unwrap_or_default()
            + "\n"
    }

    /// Salts not sent before, in a stable order.
    pub fn unsent(&self, found: &[MatchSalt]) -> Vec<MatchSalt> {
        found
            .iter()
            .filter(|s| !self.sent.contains(&s.key()))
            .copied()
            .collect()
    }

    pub fn note(&mut self, batch: &[MatchSalt], error: Option<String>, at: u64) {
        if error.is_none() {
            self.sent.extend(batch.iter().map(MatchSalt::key));
        }
        self.log.insert(
            0,
            Batch {
                at,
                matches: batch
                    .iter()
                    .map(|s| (s.match_id, s.kind.label().to_string()))
                    .collect(),
                error,
            },
        );
        self.log.truncate(LOG_KEEP);
    }

    pub fn total_sent(&self) -> usize {
        self.sent.len()
    }
}

pub fn path() -> PathBuf {
    crate::data::data_dir().join(FILE)
}

pub fn load() -> Record {
    if rune::is_headless() {
        return Record::default();
    }
    fs::read_to_string(path())
        .map(|t| Record::from_json(&t))
        .unwrap_or_default()
}

fn save(record: &Record) {
    if rune::is_headless() {
        return;
    }
    let path = path();
    let partial = path.with_extension("json.tmp");
    if fs::write(&partial, record.to_json()).is_ok() {
        let _ = fs::rename(&partial, &path);
    }
}

/// What one run did.
#[derive(Clone, Debug)]
pub struct Run {
    pub files: usize,
    pub found: usize,
    pub record: Record,
}

#[derive(Default)]
pub struct Sharing {
    pub job: Job<Run>,
    pub record: Option<Record>,
    pub last_run: Option<Instant>,
    pub last: Option<Result<(usize, usize), String>>,
}

/// Steam roots to look in: the platform defaults and the one holding the game.
fn steam_roots(st: &State) -> Vec<PathBuf> {
    let mut roots = deadlock_core::discovery::default_steam_roots();
    if let Some(game) = &st.game_root {
        // <root>/steamapps/common/Deadlock
        if let Some(root) = game.ancestors().nth(3) {
            roots.push(root.to_path_buf());
        }
    }
    roots
}

/// Scan and send now.
pub fn run_now(st: &mut State) {
    if st.settings.sharing.job.is_running() || rune::is_headless() {
        return;
    }
    let roots = steam_roots(st);
    let record = st.settings.sharing.record.clone().unwrap_or_else(load);
    let wake = st.wake.clone();
    st.settings.sharing.last_run = Some(Instant::now());
    st.settings.sharing.job.start(wake, move || {
        let scan = salts::scan(&salts::cache_dirs(&roots));
        let mut record = record;
        let unsent = record.unsent(&scan.salts);
        for chunk in unsent.chunks(BATCH) {
            let result = salts::send(chunk);
            let error = result.err();
            let failed = error.is_some();
            record.note(chunk, error, crate::screens::settings::discord::now());
            if failed {
                break;
            }
        }
        save(&record);
        Ok(Run {
            files: scan.files,
            found: scan.salts.len(),
            record,
        })
    });
}

/// Drop the log and the sent list from disk and memory.
pub fn forget(st: &mut State) {
    let _ = fs::remove_file(path());
    st.settings.sharing.record = Some(Record::default());
    st.settings.sharing.last = None;
}

/// Called from the settings poll: collect a finished run, start the next one when due.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.settings.sharing.job.try_take() {
        match result {
            Ok(run) => {
                let newest_error = run.record.log.first().and_then(|b| b.error.clone());
                let sent_now = run.record.total_sent()
                    - st.settings.sharing.record.as_ref().map_or(0, Record::total_sent).min(run.record.total_sent());
                super::log::info(format!(
                    "Match sharing: {} cache files, {} matches found, {} sent",
                    run.files, run.found, sent_now
                ));
                st.settings.sharing.last = Some(match newest_error {
                    Some(error) if sent_now == 0 && run.found > 0 => Err(error),
                    _ => Ok((run.found, sent_now)),
                });
                st.settings.sharing.record = Some(run.record);
            }
            Err(error) => st.settings.sharing.last = Some(Err(error)),
        }
        changed = true;
    }
    if st.prefs.share_matches && !st.prefs.offline && !st.demo {
        let every = std::time::Duration::from_secs(st.prefs.share_minutes.max(5) as u64 * 60);
        if st.settings.sharing.last_run.is_none_or(|t| t.elapsed() >= every) {
            run_now(st);
        }
    }
    changed
}

/// A filled log for the snapshot scene.
pub fn demo_record() -> Record {
    use deadlock_core::salts::SaltKind;
    let salt = |match_id, kind| MatchSalt {
        match_id,
        cluster_id: 404,
        kind,
        salt: 1,
    };
    let mut record = Record::default();
    let now = super::discord::now();
    record.note(
        &(0..11)
            .map(|i| salt(41_208_330 + i * 17, SaltKind::Metadata))
            .collect::<Vec<_>>(),
        None,
        now - 86_400,
    );
    record.note(
        &[salt(41_210_001, SaltKind::Replay)],
        Some("deadlock-api answered 500".into()),
        now - 3_600,
    );
    record.note(
        &[salt(41_210_001, SaltKind::Replay), salt(41_210_044, SaltKind::Metadata), salt(41_210_090, SaltKind::Metadata)],
        None,
        now - 600,
    );
    record
}

#[cfg(test)]
mod tests {
    use super::*;
    use deadlock_core::salts::SaltKind;

    fn salt(id: u64, kind: SaltKind) -> MatchSalt {
        MatchSalt {
            match_id: id,
            cluster_id: 1,
            kind,
            salt: 9,
        }
    }

    #[test]
    fn sent_salts_are_not_sent_again_and_failures_are_retried() {
        let mut r = Record::default();
        let found = [salt(1, SaltKind::Metadata), salt(1, SaltKind::Replay), salt(2, SaltKind::Metadata)];
        assert_eq!(r.unsent(&found).len(), 3);
        r.note(&found[..2], None, 10);
        assert_eq!(r.unsent(&found), vec![found[2]]);
        r.note(&found[2..], Some("deadlock-api answered 500".into()), 11);
        assert_eq!(r.unsent(&found).len(), 1, "a failed batch stays unsent");
        assert_eq!(r.log.len(), 2);
        assert_eq!(r.log[0].at, 11, "newest first");
    }

    #[test]
    fn the_record_round_trips() {
        let mut r = Record::default();
        r.note(&[salt(7, SaltKind::Replay)], None, 99);
        let back = Record::from_json(&r.to_json());
        assert_eq!(back, r);
        assert_eq!(Record::from_json("nope"), Record::default());
    }
}
