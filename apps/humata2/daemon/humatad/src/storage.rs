//! What this machine keeps, as the Storage page lists it (LD-13, LD-14,
//! LD-15): session clones, main clones, build caches in session folders,
//! the trash and the logs, measured when asked, and the actions that free
//! build caches and logs. Everything here does file work: call it on a
//! blocking thread.
//!
//! Access checks belong to the MachineService handlers, which call these
//! methods after the check. `GetStorage` and `TrimLogs` are for the user:
//! `Capability::MachineSettings` on `Target::Global`. `ClearBuildCache`
//! needs the right to manage the session whose cache it is:
//! `Capability::OwnSessionManage` on `Target::Session` (a user always has it).
//! The handlers also check that a `MachineId` in a request is this machine.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use humata2_grpc::Timestamp;
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::Layout;
use humata2_store::{ChannelRecord, SessionRecord, Store};
use humata2_transfers::{BuildCache, Measured, StorageError, build_caches, clear, disk_space, measure};
use pb::invalid_field::Problem;
use tonic::{Code, Status};

use crate::grant_target::timestamp;
use crate::status;
use crate::trash_service::{Bin, now_ms};

/// What `TrimLogs` keeps of each log file.
const LOG_KEEP_BYTES: u64 = 1 << 20;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

pub(crate) struct Storage {
    store: Arc<Store>,
    machine: String,
    layout: Layout,
    bin: Arc<Bin>,
    logs: PathBuf,
}

impl Storage {
    /// `channels` is the folder every channel's folder is in; `logs` is the
    /// daemon's log folder (`<state>/logs`).
    pub(crate) fn new(
        store: Arc<Store>,
        machine: String,
        channels: PathBuf,
        bin: Arc<Bin>,
        logs: PathBuf,
    ) -> Self {
        Self {
            store,
            machine,
            layout: Layout { root: channels },
            bin,
            logs,
        }
    }

    /// LD-13, LD-14: what this machine keeps, measured now, in the order the
    /// page shows it. A folder that can't be read counts as 0 bytes and is
    /// logged; it never fails the report. Blocking.
    pub(crate) fn report(&self, skips: Vec<pb::RetentionSkip>) -> Result<pb::StorageReport, Status> {
        let channels = self
            .store
            .channels()
            .map_err(|error| status::store(error, &self.machine, "list the channels"))?;
        let sessions = self.local_sessions()?;
        let names: HashMap<&str, &ChannelRecord> = channels
            .iter()
            .map(|channel| (channel.id.as_str(), channel))
            .collect();

        let mut clones = Vec::new();
        let mut caches = Vec::new();
        for session in &sessions {
            let channel = names
                .get(session.channel.as_str())
                .map_or(session.channel.as_str(), |channel| channel.name.as_str());
            let found = self.caches_of(&session.folder);
            let measured = self.measured(&session.folder);
            let cache_bytes: u64 = found.iter().map(|cache| cache.bytes).sum();
            clones.push(row(
                pb::StorageKind::SessionClones,
                format!("# {channel} \u{203a} {}", session.title),
                Some(session_entity(&session.id)),
                String::new(),
                measured.bytes.saturating_sub(cache_bytes),
                measured.last_used_unix_ms,
                pb::StorageAction::Archive,
            ));
            for cache in found {
                caches.push(row(
                    pb::StorageKind::BuildCache,
                    format!("{}: {}/", session.title, cache.path.display()),
                    Some(session_entity(&session.id)),
                    cache.path.to_string_lossy().into_owned(),
                    cache.bytes,
                    cache.last_used_unix_ms,
                    pb::StorageAction::Clear,
                ));
            }
        }

        let mut rows = clones;
        for channel in &channels {
            let main = self.layout.main(&channel.folder);
            if !main.is_dir() {
                continue;
            }
            let measured = self.measured(&main);
            let repos: Vec<&str> = channel.repos.iter().map(|repo| repo.name.as_str()).collect();
            rows.push(row(
                pb::StorageKind::MainClones,
                format!("# {}: {}", channel.name, repos.join(", ")),
                Some(pb::EntityRef {
                    entity: Some(pb::entity_ref::Entity::Channel(pb::ChannelId {
                        value: channel.id.clone(),
                    })),
                }),
                String::new(),
                measured.bytes,
                measured.last_used_unix_ms,
                pb::StorageAction::None,
            ));
        }
        rows.append(&mut caches);
        rows.extend(self.trash_row());
        let logs = self.measured(&self.logs);
        rows.push(row(
            pb::StorageKind::Logs,
            "humatad, holders".to_owned(),
            None,
            String::new(),
            logs.bytes,
            None,
            pb::StorageAction::Trim,
        ));

        let (free_bytes, total_bytes) = match disk_space(&self.layout.root) {
            Ok(space) => (space.free_bytes, space.total_bytes),
            Err(error) => {
                crate::log::say!(
                    "humatad2: couldn't measure the disk of {}: {}",
                    self.layout.root.display(),
                    status::chain(&error)
                );
                (0, 0)
            }
        };
        Ok(pb::StorageReport {
            machine: Some(pb::MachineId {
                value: self.machine.clone(),
            }),
            measured_at: Some(at(now_ms())),
            free_bytes,
            total_bytes,
            rows,
            retention_skips: skips,
        })
    }

    /// LD-15: removes one build cache of a session. `path` must be one of
    /// the caches found in the session's folder right now. A running
    /// session's cache is cleared only with `confirm_running`, since its
    /// next build is slower. Blocking.
    pub(crate) fn clear_build_cache(
        &self,
        request: &pb::ClearBuildCacheRequest,
    ) -> Result<pb::ClearBuildCacheResponse, Status> {
        let id = request
            .session
            .as_ref()
            .map(|session| session.value.as_str())
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing("session"))?;
        let session = self.local_session(id)?;
        let path = Path::new(&request.path);
        let inside = !request.path.is_empty()
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(_)));
        let found = match build_caches(&session.folder) {
            Ok(found) => found,
            Err(error) => {
                return Err(status::internal(
                    format!("couldn't look for build caches: {}", status::chain(&error)),
                    "",
                ));
            }
        };
        if !inside || !found.iter().any(|cache| cache.path == path) {
            return Err(status::invalid_field(
                "path",
                Problem::NotAllowedHere,
                format!(
                    "{:?} is not a build cache in the folder of session {id}; only a found build cache can be cleared",
                    request.path
                ),
                "pick a build cache from the Storage page and measure again if it was just removed",
            ));
        }
        if !request.confirm_running && self.running(id)? {
            return Err(status::failure(
                Code::FailedPrecondition,
                pb::Error {
                    message: format!(
                        "session {id} is running, and clearing {} makes its next build slower",
                        request.path
                    ),
                    fix: "ask the person, then send the request again with confirm_running set".to_owned(),
                    retry: None,
                    kind: None,
                },
            ));
        }
        let freed = clear(&session.folder.join(path)).map_err(|error| {
            status::internal(
                format!("couldn't clear {}: {}", request.path, status::chain(&error)),
                "",
            )
        })?;
        Ok(pb::ClearBuildCacheResponse { freed_bytes: freed })
    }

    /// Cuts each log file of this machine to its last 1 MiB. A process that
    /// keeps a log open keeps writing to the file the trim replaced, so the
    /// lines it writes until it reopens its log are lost. Blocking.
    pub(crate) fn trim_logs(&self, _request: &pb::TrimLogsRequest) -> Result<pb::TrimLogsResponse, Status> {
        let freed = humata2_transfers::trim_logs(&self.logs, LOG_KEEP_BYTES).map_err(|error| {
            status::internal(format!("couldn't trim the logs: {}", status::chain(&error)), "")
        })?;
        Ok(pb::TrimLogsResponse { freed_bytes: freed })
    }

    fn local_sessions(&self) -> Result<Vec<SessionRecord>, Status> {
        let all = self
            .store
            .sessions()
            .map_err(|error| status::store(error, &self.machine, "list the sessions"))?;
        Ok(all
            .into_iter()
            .filter(|session| session.machine.is_empty() || session.machine == self.machine)
            .collect())
    }

    fn local_session(&self, id: &str) -> Result<SessionRecord, Status> {
        match self.store.session(id) {
            Ok(Some(session)) if session.machine.is_empty() || session.machine == self.machine => Ok(session),
            Ok(_) => Err(status::not_found(
                pb::EntityKind::Session,
                id,
                format!("no session {id} on this machine"),
                "check the session id; sessions on other machines can't be named here yet",
            )),
            Err(error) => Err(status::store(error, &self.machine, "look up the session")),
        }
    }

    fn running(&self, session: &str) -> Result<bool, Status> {
        self.store
            .agent(session)
            .map(|agent| agent.is_some_and(|agent| agent.state.should_run()))
            .map_err(|error| status::store(error, &self.machine, "look up the session's agent"))
    }

    /// Never fails: what can't be measured is 0 and logged.
    fn measured(&self, path: &Path) -> Measured {
        measure(path).unwrap_or_else(|error| {
            said(&error);
            Measured {
                bytes: 0,
                last_used_unix_ms: None,
            }
        })
    }

    fn caches_of(&self, folder: &Path) -> Vec<BuildCache> {
        build_caches(folder).unwrap_or_else(|error| {
            if folder.exists() {
                said(&error);
            }
            Vec::new()
        })
    }

    fn trash_row(&self) -> Option<pb::StorageRow> {
        let items = match self.bin.get().and_then(|trash| {
            trash
                .list()
                .map_err(|error| status::internal(status::chain(&error), ""))
        }) {
            Ok(items) => items,
            Err(error) => {
                crate::log::say!("humatad2: couldn't measure the trash: {}", error.message());
                return None;
            }
        };
        let oldest = items.iter().map(|item| item.deleted_unix_ms).min()?;
        let days = (now_ms() - oldest).max(0) / DAY_MS;
        let count = items.len();
        let bytes = items.iter().map(|item| item.bytes).sum();
        Some(row(
            pb::StorageKind::Trash,
            format!(
                "{count} {} \u{b7} oldest {days} {}",
                if count == 1 { "item" } else { "items" },
                if days == 1 { "day" } else { "days" }
            ),
            None,
            String::new(),
            bytes,
            None,
            pb::StorageAction::Empty,
        ))
    }
}

fn said(error: &StorageError) {
    crate::log::say!("humatad2: couldn't measure storage: {}", status::chain(error));
}

fn session_entity(id: &str) -> pb::EntityRef {
    pb::EntityRef {
        entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
            value: id.to_owned(),
        })),
    }
}

fn at(ms: i64) -> Timestamp {
    timestamp(UNIX_EPOCH + Duration::from_millis(u64::try_from(ms).unwrap_or(0)))
}

fn row(
    kind: pb::StorageKind,
    what: String,
    entity: Option<pb::EntityRef>,
    path: String,
    bytes: u64,
    last_used_unix_ms: Option<i64>,
    action: pb::StorageAction,
) -> pb::StorageRow {
    pb::StorageRow {
        kind: kind as i32,
        what,
        entity,
        path,
        bytes,
        last_used_at: last_used_unix_ms.map(at),
        action: action as i32,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use humata2_store::{ChannelRepo, MainClone};

    use super::*;

    const MIB: usize = 1 << 20;

    struct Fixture {
        dir: tempfile::TempDir,
        storage: Storage,
    }

    fn write(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path().join("state/store")).unwrap());
        store
            .put_channel(&ChannelRecord {
                id: "ch_1".to_owned(),
                name: "payments".to_owned(),
                folder: "payments".to_owned(),
                branch_prefix: String::new(),
                default_agent: "zsh".to_owned(),
                repos: ["api", "ledger"]
                    .map(|name| ChannelRepo {
                        owner: "acme".to_owned(),
                        name: name.to_owned(),
                        main: MainClone::Ready,
                    })
                    .to_vec(),
                machines: Vec::new(),
                revision: 1,
                created_unix_ms: 0,
            })
            .unwrap();
        let channels = dir.path().join("channels");
        for (id, title) in [("se_1", "Rate limiter"), ("se_2", "Refunds")] {
            store
                .put_session(&SessionRecord {
                    id: id.to_owned(),
                    channel: "ch_1".to_owned(),
                    machine: String::new(),
                    title: title.to_owned(),
                    agent: "zsh".to_owned(),
                    folder: channels.join("payments/sessions").join(id),
                    created_unix_ms: 0,
                })
                .unwrap();
        }
        write(&channels.join("payments/sessions/se_1/api/src.rs"), MIB);
        write(&channels.join("payments/sessions/se_1/api/Cargo.toml"), 10);
        write(&channels.join("payments/sessions/se_1/api/target/big"), 3 * MIB);
        write(&channels.join("payments/sessions/se_2/api/src.rs"), 2 * MIB);
        write(&channels.join("payments/main/api/src.rs"), MIB);
        write(&dir.path().join("state/logs/humatad.log"), MIB / 2);
        let bin = Arc::new(Bin::new(dir.path().join("channels/.trash")));
        let storage = Storage::new(
            store,
            "ma_1".to_owned(),
            channels,
            bin,
            dir.path().join("state/logs"),
        );
        Fixture { dir, storage }
    }

    fn kinds(report: &pb::StorageReport) -> Vec<i32> {
        report.rows.iter().map(|row| row.kind).collect()
    }

    fn clear_request(session: &str, path: &str, confirm: bool) -> pb::ClearBuildCacheRequest {
        pb::ClearBuildCacheRequest {
            idempotency_key: None,
            session: Some(pb::SessionId {
                value: session.to_owned(),
            }),
            path: path.to_owned(),
            confirm_running: confirm,
        }
    }

    #[test]
    fn rows_come_in_the_pages_order_with_the_sizes_written() {
        let f = fixture();
        let report = f.storage.report(Vec::new()).unwrap();
        use pb::StorageKind as K;
        assert_eq!(
            kinds(&report),
            [
                K::SessionClones as i32,
                K::SessionClones as i32,
                K::MainClones as i32,
                K::BuildCache as i32,
                K::Logs as i32,
            ]
        );
        let rows = &report.rows;
        assert!(rows[0].what.contains("payments") && rows[0].what.contains("Rate limiter"));
        assert_eq!(rows[0].action, pb::StorageAction::Archive as i32);
        assert!(rows[0].last_used_at.is_some());
        assert!(rows[1].bytes >= 2 * MIB as u64 && rows[1].bytes < 3 * MIB as u64);
        assert!(rows[2].what.ends_with("api, ledger"));
        assert_eq!(rows[2].action, pb::StorageAction::None as i32);
        assert!(rows[2].bytes >= MIB as u64);
        assert_eq!(rows[3].path, "api/target");
        assert_eq!(rows[3].action, pb::StorageAction::Clear as i32);
        assert!(rows[4].bytes >= (MIB / 2) as u64);
        assert!(report.total_bytes > 0 && report.free_bytes <= report.total_bytes);
    }

    #[test]
    fn a_sessions_row_leaves_out_its_build_cache() {
        let f = fixture();
        let report = f.storage.report(Vec::new()).unwrap();
        let session = &report.rows[0];
        let cache = report
            .rows
            .iter()
            .find(|row| row.kind == pb::StorageKind::BuildCache as i32)
            .unwrap();
        assert!(session.bytes >= MIB as u64 && session.bytes < 2 * MIB as u64);
        assert!(cache.bytes >= 3 * MIB as u64);
    }

    #[test]
    fn clearing_a_cache_frees_it_and_the_next_report_no_longer_lists_it() {
        let f = fixture();
        let freed = f
            .storage
            .clear_build_cache(&clear_request("se_1", "api/target", false))
            .unwrap()
            .freed_bytes;
        assert!(freed >= 3 * MIB as u64);
        let report = f.storage.report(Vec::new()).unwrap();
        assert!(!kinds(&report).contains(&(pb::StorageKind::BuildCache as i32)));
        assert!(f.dir.path().join("channels/payments/sessions/se_1/api/src.rs").exists());
    }

    #[test]
    fn what_is_not_a_found_cache_is_refused_and_deletes_nothing() {
        let f = fixture();
        for path in ["api", "api/src.rs", "../se_2", "/etc", "", "api/../api/target", "api/nothing"] {
            let refused = f
                .storage
                .clear_build_cache(&clear_request("se_1", path, true))
                .unwrap_err();
            assert_eq!(refused.code(), Code::InvalidArgument, "{path:?}");
        }
        let session = f.dir.path().join("channels/payments/sessions");
        assert!(session.join("se_1/api/target/big").exists());
        assert!(session.join("se_1/api/src.rs").exists());
        assert!(session.join("se_2/api/src.rs").exists());
    }

    #[test]
    fn a_session_that_is_not_here_is_not_found() {
        let f = fixture();
        let refused = f
            .storage
            .clear_build_cache(&clear_request("se_none", "api/target", true))
            .unwrap_err();
        assert_eq!(refused.code(), Code::NotFound);
    }

    #[test]
    fn a_missing_session_folder_still_yields_a_report() {
        let f = fixture();
        fs::remove_dir_all(f.dir.path().join("channels/payments/sessions/se_2")).unwrap();
        let report = f.storage.report(Vec::new()).unwrap();
        let rows: Vec<_> = report
            .rows
            .iter()
            .filter(|row| row.kind == pb::StorageKind::SessionClones as i32)
            .collect();
        assert_eq!(rows.len(), 2);
        assert!(rows[1].bytes < 100 * 1024);
    }

    #[test]
    fn trimming_logs_frees_what_is_past_the_last_mebibyte() {
        let f = fixture();
        let log = f.dir.path().join("state/logs/humatad.log");
        let lines: String = (0..200_000).map(|n| format!("line {n:06}\n")).collect();
        fs::write(&log, lines).unwrap();
        let freed = f
            .storage
            .trim_logs(&pb::TrimLogsRequest {
                idempotency_key: None,
                id: None,
            })
            .unwrap()
            .freed_bytes;
        assert!(freed > 0);
        assert!(fs::metadata(&log).unwrap().len() <= MIB as u64);
    }

    #[test]
    fn retention_skips_are_passed_through() {
        let f = fixture();
        let skip = pb::RetentionSkip {
            session: Some(pb::SessionId {
                value: "se_1".to_owned(),
            }),
            reason: "unpushed work".to_owned(),
        };
        let report = f.storage.report(vec![skip.clone()]).unwrap();
        assert_eq!(report.retention_skips, [skip]);
    }
}
