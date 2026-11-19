//! What a boot does with the stop record (D261, D265): a stopped one sweeps
//! and settles the record and brings nothing back; one that a start renamed
//! brings everything back and then says in its report which stop it came
//! after.

use std::path::Path;
use std::sync::Arc;

use humata2_grpc::humata::v1 as pb;
use humata2_store::{Store, StoreError, TabKind, TabRecord};
use serde_json::{Value, json};

use super::sweep::{HolderEnd, How, Sweep};
use super::{ByKind, Ended, Record, StopFolder, StopState, StoppedBy, Via};
use crate::inbox::Inbox;
use crate::status;

/// How this boot reads the stop record.
pub(crate) enum Mode {
    /// Nothing is stopped: bring back as after any restart.
    Running,
    /// Sessions are stopped. The record, unless it can't be read: then the
    /// machine stays stopped with no record to settle.
    Stopped(Option<Record>),
    /// A start renamed the record: bring back, then name the stop.
    Starting(Record),
}

/// Reads the record. One that can't be read keeps the machine stopped:
/// launching agents is the harm a stop exists to prevent.
pub(crate) fn read_mode(folder: &StopFolder) -> Mode {
    match folder.read() {
        Ok(StopState::Running) => Mode::Running,
        Ok(StopState::Stopped(record)) => Mode::Stopped(Some(record)),
        Ok(StopState::Starting(record)) => Mode::Starting(record),
        Err(error) => {
            crate::log::say!(
                "humatad2: this machine stays stopped: {}",
                status::chain(&error)
            );
            Mode::Stopped(None)
        }
    }
}

/// The stop a start came after, for the start to name and then remove.
pub(crate) struct AfterStop {
    pub(crate) record: Record,
    pub(crate) folder: StopFolder,
    /// This machine's id, as the one who started it.
    pub(crate) machine: String,
}

/// What a stopped boot ends and settles.
pub(crate) struct Settle {
    pub(crate) folder: StopFolder,
    pub(crate) record: Option<Record>,
    pub(crate) sweep: Sweep,
    pub(crate) store: Arc<Store>,
    pub(crate) inbox: Arc<Inbox>,
}

impl Settle {
    /// Ends every holder, withdraws the Inbox's pending requests, writes
    /// what ended into the record, and returns the start to show: stopped,
    /// with no report and nothing resumed.
    ///
    /// D801 I, PER-32: these ends stay. They finish the Stop the owner
    /// chose (`humatad2 stop`, or Stop in the app) before this boot; a
    /// boot without a stop record never comes here and ends nothing.
    pub(crate) async fn run(self) -> pb::LastStart {
        let Settle {
            folder,
            record,
            sweep,
            store,
            inbox,
        } = self;
        let withdrawing = async {
            let done = inbox
                .withdraw_all_pending(pb::WithdrawReason::MachineStopped)
                .await;
            crate::fault::point("stop.inbox.withdrawn");
            done
        };
        let (swept, withdrawn) = tokio::join!(sweep.run(), withdrawing);
        if let Err(error) = withdrawn {
            crate::log::say!(
                "humatad2: couldn't withdraw the Inbox's pending requests for the stop: {}",
                status::chain(&error)
            );
        }
        let settled = match swept {
            Ok(mut swept) => {
                // What `humatad2 stop` ended itself, for a daemon that
                // didn't answer, counts as this stop's too.
                let noted = folder.clone();
                match tokio::task::spawn_blocking(move || noted.ended_early()).await {
                    Ok(early) => swept.ended.extend(early),
                    Err(error) => crate::log::say!("humatad2: a background task failed: {error}"),
                }
                let tabs = match tokio::task::spawn_blocking(move || store.tabs()).await {
                    Ok(tabs) => tabs,
                    Err(error) => {
                        crate::log::say!("humatad2: a background task failed: {error}");
                        Ok(Vec::new())
                    }
                };
                settle_record(&folder, record, &swept, tabs).await
            }
            Err(error) => {
                crate::log::say!(
                    "humatad2: couldn't end the sessions' processes for the stop; stop again once that is fixed: {}",
                    status::chain(&error)
                );
                record
            }
        };
        pb::LastStart {
            started_at: Some(crate::attach::now()),
            sessions_stopped: Some(settled.as_ref().map(to_pb).unwrap_or_default()),
            ..pb::LastStart::default()
        }
    }
}

/// Writes what the sweep ended into the record, on top of what an earlier
/// sweep of the same stop ended. A record that can't be settled is logged
/// and shown as it stands.
async fn settle_record(
    folder: &StopFolder,
    record: Option<Record>,
    swept: &super::sweep::Swept,
    tabs: Result<Vec<TabRecord>, StoreError>,
) -> Option<Record> {
    let record = record?;
    let tabs = tabs.unwrap_or_else(|error| {
        crate::log::say!(
            "humatad2: couldn't read the tabs to count what the stop ended: {}",
            status::chain(&error)
        );
        Vec::new()
    });
    let ended = ended_of(record.ended.as_ref(), &swept.ended, &swept.left, &tabs);
    let folder = folder.clone();
    let before = record.clone();
    let settled = tokio::task::spawn_blocking(move || {
        let settled = folder.settle(&ended)?;
        if let Err(error) = folder.clear_ended_early() {
            crate::log::say!(
                "humatad2: couldn't remove the note of holders `humatad2 stop` ended: {}",
                status::chain(&error)
            );
        }
        Ok::<_, super::RecordError>(settled)
    })
    .await;
    match settled {
        Ok(Ok(settled)) => Some(settled),
        Ok(Err(error)) => {
            crate::log::say!(
                "humatad2: couldn't save what the stop ended: {}",
                status::chain(&error)
            );
            Some(before)
        }
        Err(error) => {
            crate::log::say!("humatad2: a background task failed: {error}");
            Some(before)
        }
    }
}

/// What a stop ended: the holders this sweep ended, counted by the kind of
/// tab each one held, added to what an earlier sweep of the same stop
/// ended. `left` is what is still alive now.
pub(crate) fn ended_of(
    earlier: Option<&Ended>,
    holders: &[HolderEnd],
    left: &[u32],
    tabs: &[TabRecord],
) -> Ended {
    let mut now = Ended {
        agents: 0,
        shells: 0,
        killed: 0,
        left: left.to_vec(),
    };
    for holder in holders {
        match tabs.iter().find(|tab| tab.holder_socket == holder.socket) {
            Some(tab) if tab.kind == TabKind::Agent => now.agents += 1,
            Some(_) => now.shells += 1,
            None => {}
        }
        if holder.how == How::Killed {
            now.killed += 1;
        }
    }
    if let Some(earlier) = earlier {
        now.agents += earlier.agents;
        now.shells += earlier.shells;
        now.killed += earlier.killed;
    }
    now
}

/// The record as the API says it.
pub(crate) fn to_pb(record: &Record) -> pb::SessionsStopped {
    pb::SessionsStopped {
        stop_id: record.stop_id.clone(),
        stopped_at_unix_ms: record.stopped_at_unix_ms,
        by: Some(by_pb(&record.by)),
        reason: record.reason.clone().unwrap_or_default(),
        ended: record.ended.as_ref().map(|ended| pb::StopEnded {
            agents: ended.agents,
            shells: ended.shells,
            killed: ended.killed,
            left: u32::try_from(ended.left.len()).unwrap_or(u32::MAX),
        }),
    }
}

fn by_pb(by: &StoppedBy) -> pb::StoppedBy {
    pb::StoppedBy {
        kind: match by.kind {
            ByKind::User => pb::StoppedByKind::User,
            ByKind::Agent => pb::StoppedByKind::Agent,
            ByKind::Terminal => pb::StoppedByKind::Terminal,
            ByKind::Script => pb::StoppedByKind::Script,
        } as i32,
        via: match by.via {
            Via::App => pb::StopVia::App,
            Via::Cli => pb::StopVia::Cli,
            Via::Ssh => pb::StopVia::Ssh,
            Via::Direct => pb::StopVia::Direct,
        } as i32,
        machine: by.machine.clone(),
        session: by.session.clone().unwrap_or_default(),
    }
}

/// Finishes a start that came after the stop `record`: says so in the
/// restart report and in the start shown, then removes the record. Every
/// step that fails is logged and the rest go on: the sessions are back.
pub(crate) async fn finish_start(mut start: pb::LastStart, after: AfterStop) -> pb::LastStart {
    let AfterStop {
        record,
        folder,
        machine,
    } = after;
    let started_by = StoppedBy {
        kind: ByKind::User,
        via: Via::Direct,
        machine,
        session: None,
    };
    if !start.report.is_empty() {
        let report = Path::new(&start.report).join("restart.json");
        let named = after_stop_json(&record, &started_by);
        match tokio::task::spawn_blocking(move || name_stop_in(&report, &named)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => crate::log::say!(
                "humatad2: couldn't name the stop in the restart report: {}",
                status::chain(&error)
            ),
            Err(error) => crate::log::say!("humatad2: a background task failed: {error}"),
        }
    }
    crate::fault::point("start.report.written");
    start.after_stop = Some(pb::AfterStop {
        stop: Some(to_pb(&record)),
        started_by: Some(by_pb(&started_by)),
    });
    match tokio::task::spawn_blocking(move || folder.finish_start()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => crate::log::say!(
            "humatad2: couldn't remove the stop record after the start: {}",
            status::chain(&error)
        ),
        Err(error) => crate::log::say!("humatad2: a background task failed: {error}"),
    }
    start
}

/// What `restart.json` says about the stop a start came after.
fn after_stop_json(record: &Record, started_by: &StoppedBy) -> Value {
    let stop = serde_json::from_slice::<Value>(&record.to_json()).unwrap_or(Value::Null);
    json!({
        "stop": stop,
        "started_by": {"kind": "user", "machine": started_by.machine},
    })
}

#[derive(Debug, thiserror::Error)]
enum NameStopError {
    #[error("couldn't read the restart report {}; check that it belongs to you", path.display())]
    Read {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the restart report {} isn't a JSON object, so the stop wasn't named in it", path.display())]
    NotAnObject { path: std::path::PathBuf },
    #[error("couldn't write the restart report {}; check that the disk has room", path.display())]
    Write {
        path: std::path::PathBuf,
        #[source]
        source: StoreError,
    },
}

/// Adds `after_stop` to the `restart.json` at `path`. Blocks.
fn name_stop_in(path: &Path, after_stop: &Value) -> Result<(), NameStopError> {
    let bytes = std::fs::read(path).map_err(|source| NameStopError::Read {
        path: path.to_owned(),
        source,
    })?;
    let mut report: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let Some(object) = report.as_object_mut() else {
        return Err(NameStopError::NotAnObject {
            path: path.to_owned(),
        });
    };
    object.insert("after_stop".to_owned(), after_stop.clone());
    let text = serde_json::to_string_pretty(&report).unwrap_or_default();
    humata2_store::write_atomic(path, text.as_bytes()).map_err(|source| NameStopError::Write {
        path: path.to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;

    fn tab(id: &str, kind: TabKind) -> TabRecord {
        TabRecord {
            id: id.to_owned(),
            session: Some("se_1".to_owned()),
            kind,
            title: String::new(),
            holder_socket: PathBuf::from(format!("/h/{id}.sock")),
            holder_pid: 1,
            created_unix_ms: 0,
            holder_generation: 0,
        }
    }

    fn holder(id: &str, how: How) -> HolderEnd {
        HolderEnd {
            socket: PathBuf::from(format!("/h/{id}.sock")),
            pid: 10,
            how,
        }
    }

    fn record() -> Record {
        Record {
            stop_id: "stop-1".to_owned(),
            stopped_at_unix_ms: 5,
            by: StoppedBy {
                kind: ByKind::User,
                via: Via::App,
                machine: "ma_1".to_owned(),
                session: None,
            },
            reason: None,
            ended: None,
        }
    }

    #[test]
    fn holders_are_counted_by_the_kind_of_tab_they_held_and_the_killed_ones_too() {
        let tabs = [tab("a", TabKind::Agent), tab("b", TabKind::Shell), tab("c", TabKind::Shell)];
        let holders = [holder("a", How::Ended), holder("b", How::Killed), holder("c", How::Ended)];
        let ended = ended_of(None, &holders, &[], &tabs);
        assert_eq!((ended.agents, ended.shells, ended.killed), (1, 2, 1));
    }

    #[test]
    fn a_second_sweep_of_the_same_stop_adds_to_what_the_first_ended() {
        let earlier = Ended { agents: 1, shells: 1, killed: 1, left: vec![9] };
        let ended = ended_of(Some(&earlier), &[], &[], &[]);
        assert_eq!((ended.agents, ended.shells, ended.killed), (1, 1, 1));
        assert!(ended.left.is_empty());
    }

    #[test]
    fn what_is_left_alive_is_what_this_sweep_found() {
        let ended = ended_of(None, &[], &[42], &[]);
        assert_eq!(ended.left, vec![42]);
    }

    #[test]
    fn a_folder_with_no_record_is_running() {
        let dir = TempDir::new().expect("a temp dir");
        assert!(matches!(read_mode(&StopFolder::new(dir.path())), Mode::Running));
    }

    #[test]
    fn a_stop_record_that_reads_is_stopped_with_its_record() {
        let dir = TempDir::new().expect("a temp dir");
        let folder = StopFolder::new(dir.path());
        folder.write_stopped(&record()).expect("saved");
        assert!(matches!(read_mode(&folder), Mode::Stopped(Some(found)) if found == record()));
    }

    #[test]
    fn a_stop_record_that_does_not_read_keeps_the_machine_stopped() {
        let dir = TempDir::new().expect("a temp dir");
        std::fs::create_dir_all(dir.path().join("stop")).expect("folder");
        std::fs::write(dir.path().join("stop/this.json"), b"\x00 damaged").expect("file");
        assert!(matches!(read_mode(&StopFolder::new(dir.path())), Mode::Stopped(None)));
    }

    #[test]
    fn a_renamed_record_is_a_start() {
        let dir = TempDir::new().expect("a temp dir");
        let folder = StopFolder::new(dir.path());
        folder.write_stopped(&record()).expect("saved");
        folder.begin_start().expect("started");
        assert!(matches!(read_mode(&folder), Mode::Starting(found) if found == record()));
    }

    #[test]
    fn the_stop_is_named_in_a_restart_report_and_its_other_fields_stay() {
        let dir = TempDir::new().expect("a temp dir");
        let path = dir.path().join("restart.json");
        std::fs::write(&path, br#"{"format":1,"machine":"m"}"#).expect("report");
        let by = record().by;
        name_stop_in(&path, &after_stop_json(&record(), &by)).expect("named");
        let report: Value = serde_json::from_slice(&std::fs::read(&path).expect("read")).expect("json");
        assert_eq!(report["machine"], "m");
        assert!(report["after_stop"].to_string().contains("stop-1"));
    }

    #[test]
    fn a_report_that_is_not_an_object_is_left_alone() {
        let dir = TempDir::new().expect("a temp dir");
        let path = dir.path().join("restart.json");
        std::fs::write(&path, b"[1]").expect("report");
        let by = record().by;
        assert!(name_stop_in(&path, &after_stop_json(&record(), &by)).is_err());
        assert_eq!(std::fs::read(&path).expect("read"), b"[1]");
    }

    #[test]
    fn a_stop_converts_to_the_api_with_its_id_and_what_it_ended() {
        let mut stopped = record();
        stopped.ended = Some(Ended { agents: 2, shells: 3, killed: 1, left: vec![] });
        let shown = to_pb(&stopped);
        assert_eq!(shown.stop_id, "stop-1");
        let ended = shown.ended.expect("ended");
        assert_eq!((ended.agents, ended.shells, ended.killed, ended.left), (2, 3, 1, 0));
        assert!(to_pb(&record()).ended.is_none());
    }
}
