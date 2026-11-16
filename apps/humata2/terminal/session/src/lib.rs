//! Persistent local/SSH PTY sessions, independent of any UI or application model.
//! Dropping an attachment never stops its program. Stopping is explicit.
mod error;
mod identity;
mod journal;
mod metadata;
mod ssh;

pub use error::Error;
use error::disk;
use humata2_holder::{Attachment, Control, Event, Events, HolderSpec};
pub use journal::Record;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::{
    fs::{self, File, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::fs::{OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
pub use terminal_wire::RecoveryHint;
pub use terminal_wire::{NameSource, SessionNameOverride};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Endpoint {
    Local,
    /// Uses the user's SSH host-key/authentication configuration. Install a matching holder first.
    Ssh {
        config: PathBuf,
        destination: String,
        holder: PathBuf,
        root: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSpec {
    pub title: String,
    pub program: PathBuf,
    /// Launch-only arguments may contain secrets; never persist them.
    #[serde(default, skip_serializing)]
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub cols: u16,
    pub rows: u16,
    pub endpoint: Endpoint,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedSession {
    pub id: u64,
    #[serde(default)]
    pub uuid: String,
    #[serde(default)]
    pub automatic_title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_override: Option<SessionNameOverride>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, String>,
    pub spec: SessionSpec,
    #[serde(default)]
    pub recovery: Option<RecoveryHint>,
    /// Stable launcher identity, persisted with the descriptor before launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_profile: Option<String>,
}
impl SavedSession {
    pub fn display_title(&self) -> &str {
        self.name_override
            .as_ref()
            .map_or(&self.automatic_title, |name| &name.title)
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Saved {
    namespace: String,
    version: u32,
    next: u64,
    sessions: Vec<SavedSession>,
    groups: [Vec<u64>; 2],
    #[serde(default)]
    selected: [Option<u64>; 2],
}

/// One owner of the workspace metadata. Moving views changes layout, never the PTY.
pub struct SessionStore {
    root: PathBuf,
    holder: PathBuf,
    saved: Saved,
    memory_only: bool,
    _lock: File,
}
impl Drop for SessionStore {
    fn drop(&mut self) {
        // A concurrently spawned child may briefly retain the same open-file
        // description before exec closes its CLOEXEC descriptors. Closing our
        // File alone would then delay release beyond this logical owner's
        // lifetime. Explicit unlock releases ownership before the fd closes.
        // SAFETY: _lock remains owned and valid until after this Drop body.
        let _ = unsafe { libc::flock(self._lock.as_raw_fd(), libc::LOCK_UN) };
    }
}
impl SessionStore {
    pub fn open(root: &Path, holder: PathBuf) -> Result<Self, Error> {
        let existed = root.exists();
        fs::create_dir_all(root).map_err(|e| disk("create", root, e))?;
        if existed {
            let mode = fs::metadata(root)
                .map_err(|e| disk("inspect", root, e))?
                .permissions()
                .mode();
            if mode & 0o077 != 0 {
                return Err(Error::Invalid{reason:"the existing state folder is accessible by other users; choose a dedicated private folder instead of changing a shared directory".into()});
            }
        } else {
            fs::set_permissions(root, fs::Permissions::from_mode(0o700))
                .map_err(|e| disk("protect", root, e))?;
        }

        let root = fs::canonicalize(root).map_err(|e| disk("locate", root, e))?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(root.join("lock"))
            .map_err(|e| disk("lock", &root, e))?;
        // SAFETY: lock owns this valid fd until the store is dropped. flock is advisory.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(Error::Locked {
                path: root,
                source: std::io::Error::last_os_error(),
            });
        }
        let manifest = root.join("workspace.json");
        let mut saved = match metadata::read(&manifest)? {
            Some(saved) => saved,
            None => Saved {
                namespace: namespace()?,
                version: 3,
                next: 1,
                sessions: Vec::new(),
                groups: [Vec::new(), Vec::new()],
                selected: [None, None],
            },
        };
        let migrate = saved.version < 3;
        if migrate {
            for session in &mut saved.sessions {
                if session.uuid.is_empty() {
                    session.uuid = identity::new_uuid()?;
                }
                if session.automatic_title.is_empty() {
                    session.automatic_title =
                        identity::legacy_title(&session.spec.title, session.id);
                }
            }
        }
        saved.version = 3;
        let store = Self {
            root,
            holder,
            saved,
            memory_only: false,
            _lock: lock,
        };
        if migrate {
            store.save()?;
        }
        Ok(store)
    }
    /// The persistent host owns memory and bounded checkpoints itself. This
    /// mode writes only launch/recovery metadata and zero-content lock files;
    /// holder replay remains in memory, not an additional raw disk transcript.
    pub fn open_memory_only(root: &Path, holder: PathBuf) -> Result<Self, Error> {
        let mut store = Self::open(root, holder)?;
        for entry in fs::read_dir(&store.root).map_err(|e| disk("inspect", &store.root, e))? {
            let entry = entry.map_err(|e| disk("inspect", &store.root, e))?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('s')
                && (name.ends_with(".journal") || name.ends_with(".scrollback"))
            {
                return Err(Error::Invalid{reason:"this workspace contains legacy raw history; choose a fresh private host folder or explicitly archive/remove that history before switching modes".into()});
            }
        }
        store.memory_only = true;
        Ok(store)
    }
    pub fn set_recovery(&mut self, id: u64, hint: Option<RecoveryHint>) -> Result<(), Error> {
        if let Some(hint) = &hint {
            let valid = |text: &str, max: usize| {
                !text.is_empty() && text.len() <= max && !text.chars().any(char::is_control)
            };
            if !valid(&hint.provider, 64)
                || !valid(&hint.session_id, 512)
                || !valid(&hint.working_directory, 4096)
                || hint.state_paths.len() > 16
                || hint.state_paths.iter().any(|p| !valid(p, 4096))
            {
                return Err(Error::Invalid{reason:"recovery metadata needs a provider, session ID and working directory without control characters, plus at most sixteen state paths".into()});
            }
        }
        let before = self.saved.clone();
        let session = self
            .saved
            .sessions
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or(Error::Missing { id })?;
        session.recovery = hint;
        if let Err(e) = self.save() {
            self.saved = before;
            return Err(e);
        }
        Ok(())
    }
    /// A known path supplied by the launcher or the process's OSC 7 event.
    pub fn set_cwd(&mut self, id: u64, cwd: PathBuf) -> Result<(), Error> {
        if !cwd.is_absolute() {
            return Err(Error::Invalid {
                reason: "the recorded working directory must be absolute".into(),
            });
        }
        let before = self.saved.clone();
        let session = self
            .saved
            .sessions
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or(Error::Missing { id })?;
        if session.spec.cwd == cwd {
            return Ok(());
        }
        session.spec.cwd = cwd;
        if let Err(e) = self.save() {
            self.saved = before;
            return Err(e);
        }
        Ok(())
    }
    pub fn sessions(&self) -> &[SavedSession] {
        &self.saved.sessions
    }
    pub fn groups(&self) -> &[Vec<u64>; 2] {
        &self.saved.groups
    }
    pub fn selected(&self) -> [Option<u64>; 2] {
        self.saved.selected
    }
    pub fn select(&mut self, group: usize, id: u64) -> Result<(), Error> {
        if group > 1 || !self.saved.groups[group].contains(&id) {
            return Err(Error::Invalid {
                reason: "select a terminal in its current group".into(),
            });
        }
        let previous = self.saved.selected;
        self.saved.selected[group] = Some(id);
        if let Err(e) = self.save() {
            self.saved.selected = previous;
            return Err(e);
        }
        Ok(())
    }
    pub fn session(&self, id: u64) -> Result<&SavedSession, Error> {
        self.saved
            .sessions
            .iter()
            .find(|s| s.id == id)
            .ok_or(Error::Missing { id })
    }
    pub fn session_by_uuid(&self, uuid: &str) -> Result<&SavedSession, Error> {
        identity::validate_uuid(uuid)?;
        self.saved
            .sessions
            .iter()
            .find(|session| session.uuid == uuid)
            .ok_or_else(|| Error::MissingUuid { uuid: uuid.into() })
    }
    /// Runtime/agent automatic updates never change an explicit name override.
    pub fn set_automatic_title(&mut self, id: u64, title: String) -> Result<(), Error> {
        identity::validate_title(id, &title)?;
        if self.session(id)?.automatic_title == title {
            return Ok(());
        }
        self.update_identity(id, |session| session.automatic_title = title)
    }
    /// Explicit caller-authorized names use last-writer-wins semantics.
    pub fn set_name_override(
        &mut self,
        id: u64,
        title: String,
        source: NameSource,
    ) -> Result<(), Error> {
        identity::validate_title(id, &title)?;
        let name = Some(SessionNameOverride { title, source });
        if self.session(id)?.name_override == name {
            return Ok(());
        }
        self.update_identity(id, |session| session.name_override = name)
    }
    pub fn clear_name_override(&mut self, id: u64) -> Result<(), Error> {
        if self.session(id)?.name_override.is_none() {
            return Ok(());
        }
        self.update_identity(id, |session| session.name_override = None)
    }
    /// Replace explicit non-secret wrapper metadata. Nothing is inferred from
    /// process argv, environment, provider transcripts or terminal input.
    pub fn set_attributes(
        &mut self,
        id: u64,
        attributes: BTreeMap<String, String>,
    ) -> Result<(), Error> {
        identity::validate_attributes(id, &attributes)?;
        if self.session(id)?.attributes == attributes {
            return Ok(());
        }
        self.update_identity(id, |session| session.attributes = attributes)
    }
    fn update_identity(
        &mut self,
        id: u64,
        edit: impl FnOnce(&mut SavedSession),
    ) -> Result<(), Error> {
        let before = self.saved.clone();
        edit(
            self.saved
                .sessions
                .iter_mut()
                .find(|session| session.id == id)
                .ok_or(Error::Missing { id })?,
        );
        if let Err(error) = self.save() {
            self.saved = before;
            return Err(error);
        }
        Ok(())
    }
    /// Creates a new process; reconnecting to an existing process uses `attach` instead.
    pub fn create(&mut self, spec: SessionSpec) -> Result<u64, Error> {
        self.create_inner(spec, None)
    }

    /// Reserve a launcher identity before starting its process. Retrying the
    /// same key returns its existing ID, including a failed or exited launch;
    /// it never starts a replacement process or changes that session's spec.
    pub fn create_startup(&mut self, spec: SessionSpec, profile: &str) -> Result<u64, Error> {
        metadata::validate_startup_key(profile, &self.root.join("workspace.json"))?;
        if let Some(session) = self
            .saved
            .sessions
            .iter()
            .find(|session| session.startup_profile.as_deref() == Some(profile))
        {
            return Ok(session.id);
        }
        self.create_inner(spec, Some(profile.into()))
    }

    /// Bind an existing legacy session to its launcher without spawning it.
    /// A session/key already bound to another identity is never overwritten.
    pub fn bind_startup_profile(&mut self, id: u64, profile: &str) -> Result<(), Error> {
        let path = self.root.join("workspace.json");
        metadata::validate_startup_key(profile, &path)?;
        self.session(id)?;
        if self
            .saved
            .sessions
            .iter()
            .any(|session| session.id != id && session.startup_profile.as_deref() == Some(profile))
        {
            return Err(Error::MetadataStructure {
                path,
                reason: "the startup profile is already bound to another session",
            });
        }
        let current = self.session(id)?.startup_profile.as_deref();
        if current == Some(profile) {
            return Ok(());
        }
        if current.is_some() {
            return Err(Error::MetadataStructure {
                path,
                reason: "the session is already bound to another startup profile",
            });
        }
        let before = self.saved.clone();
        self.saved
            .sessions
            .iter_mut()
            .find(|session| session.id == id)
            .ok_or(Error::Missing { id })?
            .startup_profile = Some(profile.into());
        if let Err(error) = self.save() {
            self.saved = before;
            return Err(error);
        }
        Ok(())
    }

    fn create_inner(
        &mut self,
        spec: SessionSpec,
        startup_profile: Option<String>,
    ) -> Result<u64, Error> {
        metadata::before_create(&self.saved, &self.root.join("workspace.json"))?;
        if spec.cols == 0 || spec.rows == 0 || spec.cols > 1000 || spec.rows > 1000 {
            return Err(Error::Invalid {
                reason: "terminal dimensions must be between 1 and 1000 cells".into(),
            });
        }
        if let Endpoint::Ssh {
            root, destination, ..
        } = &spec.endpoint
        {
            if !root.starts_with('/')
                || root.contains([':', '\n'])
                || destination.starts_with('-')
                || destination.is_empty()
            {
                return Err(Error::Invalid{reason:"SSH needs a host alias and an absolute remote state folder without colons or newlines".into()});
            }
        }
        let before = self.saved.clone();
        let id = self.saved.next;
        identity::validate_title(id, &spec.title)?;
        let uuid = identity::new_uuid()?;
        self.saved.next += 1;
        self.saved.sessions.push(SavedSession {
            id,
            uuid,
            automatic_title: spec.title.clone(),
            name_override: None,
            attributes: BTreeMap::new(),
            spec: spec.clone(),
            recovery: None,
            startup_profile,
        });
        self.saved.groups[0].push(id);
        self.saved.selected[0] = Some(id);
        if let Err(e) = self.save() {
            self.saved = before;
            return Err(e);
        }
        let socket = self.socket(id);
        match &spec.endpoint {
            Endpoint::Local => {
                let spawn = if self.memory_only {
                    humata2_holder::spawn_memory_only
                } else {
                    humata2_holder::spawn
                };
                spawn(&HolderSpec {
                    holder: self.holder.clone(),
                    socket,
                    program: spec.program.clone(),
                    args: spec.args.iter().map(Into::into).collect(),
                    cwd: spec.cwd.clone(),
                    env: vec![
                        ("TERM".into(), "xterm-256color".into()),
                        ("COLORTERM".into(), "truecolor".into()),
                    ],
                    cols: spec.cols,
                    rows: spec.rows,
                    scrollback_bytes: 16 << 20,
                    log: self.root.join(format!("s{id}.log")),
                })
                .map_err(|source| Error::Holder {
                    id,
                    operation: "start",
                    source,
                })?;
            }
            endpoint => ssh::start(
                &self.endpoint(endpoint),
                &format!("s{id}.sock"),
                &spec,
                self.memory_only,
            )?,
        };
        Ok(id)
    }
    pub fn move_to(&mut self, id: u64, group: usize, index: usize) -> Result<(), Error> {
        self.session(id)?;
        if group > 1 {
            return Err(Error::Invalid {
                reason: "the experiment has two groups".into(),
            });
        }
        let before = self.saved.clone();
        for g in &mut self.saved.groups {
            g.retain(|x| *x != id);
        }
        let index = index.min(self.saved.groups[group].len());
        self.saved.groups[group].insert(index, id);
        for g in 0..2 {
            if !self.saved.selected[g]
                .is_some_and(|selected| self.saved.groups[g].contains(&selected))
            {
                self.saved.selected[g] = self.saved.groups[g].first().copied();
            }
        }
        self.saved.selected[group] = Some(id);
        if let Err(e) = self.save() {
            self.saved = before;
            return Err(e);
        }
        Ok(())
    }
    fn endpoint(&self, endpoint: &Endpoint) -> Endpoint {
        match endpoint {
            Endpoint::Local => Endpoint::Local,
            Endpoint::Ssh {
                config,
                destination,
                holder,
                root,
            } => Endpoint::Ssh {
                config: config.clone(),
                destination: destination.clone(),
                holder: holder.clone(),
                root: format!("{root}/{}", self.saved.namespace),
            },
        }
    }
    fn socket(&self, id: u64) -> PathBuf {
        self.root.join(format!("s{id}.sock"))
    }
    fn save(&self) -> Result<(), Error> {
        let dest = self.root.join("workspace.json");
        let tmp = self.root.join("workspace.tmp");
        let bytes = metadata::encode(&self.saved, &dest)?;
        use std::io::Write;
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&tmp)
            .map_err(|e| disk("write", &tmp, e))?;
        metadata::check_output_file(&f, &tmp)?;
        f.set_len(0)
            .map_err(|e| disk("truncate metadata temporary file", &tmp, e))?;
        f.write_all(&bytes)
            .and_then(|_| f.sync_all())
            .map_err(|e| disk("sync", &tmp, e))?;
        fs::rename(&tmp, &dest)
            .and_then(|_| File::open(&self.root)?.sync_all())
            .map_err(|e| disk("commit", &dest, e))
    }
    /// Restores the journal and attaches without starting a replacement shell.
    pub async fn attach(&self, id: u64) -> Result<Attached, Error> {
        self.attach_at(id, None).await
    }
    /// Prepare a capability for reconnecting to an existing memory-only holder.
    /// The capability owns no process, connection or workspace owner lock, and
    /// carries no launch arguments. Preparing or attaching never starts a replacement.
    pub fn prepare_live_attachment(&self, id: u64) -> Result<LiveAttacher, Error> {
        if !self.memory_only {
            return Err(Error::Invalid { reason: "offset-based host attachment requires a memory-only SessionStore; journaled views must restore their journal".into() });
        }
        let session = self.session(id)?;
        Ok(LiveAttacher {
            id,
            endpoint: self.endpoint(&session.spec.endpoint),
            lock: self.root.join(format!("s{id}.attachment.lock")),
            socket: match session.spec.endpoint {
                Endpoint::Local => self.socket(id),
                _ => self.root.join(format!("t{id}-{}.sock", std::process::id())),
            },
        })
    }
    /// Attach to the live holder after a host-owned VT checkpoint's offset.
    pub async fn attach_live_from(&self, id: u64, offset: u64) -> Result<Attached, Error> {
        self.prepare_live_attachment(id)?.attach_from(offset).await
    }
    async fn attach_at(&self, id: u64, from: Option<u64>) -> Result<Attached, Error> {
        if self.memory_only {
            return self
                .prepare_live_attachment(id)?
                .attach_from(from.unwrap_or(0))
                .await;
        }
        let session = self.session(id)?;
        let (journal, history, offset) =
            journal::Journal::open(&self.root.join(format!("s{id}.journal")))?;
        let socket = match session.spec.endpoint {
            Endpoint::Local => self.socket(id),
            _ => self.root.join(format!("t{id}-{}.sock", std::process::id())),
        };
        attach_existing(
            id,
            self.endpoint(&session.spec.endpoint),
            socket,
            journal,
            history,
            offset,
        )
        .await
    }
    /// Reads saved display history while detached, even when the process or remote host is unavailable.
    pub fn history(&self, id: u64) -> Result<Vec<Record>, Error> {
        self.session(id)?;
        if self.memory_only {
            return Ok(Vec::new());
        }
        Ok(journal::Journal::open(&self.root.join(format!("s{id}.journal")))?.1)
    }
    /// Stops the live process explicitly. Saved display history and layout remain.
    pub async fn stop(&self, id: u64) -> Result<(), Error> {
        let mut a = self.attach(id).await?;
        a.writer.stop().await?;
        tokio::time::timeout(Duration::from_secs(5), async {
            while a.reader.next().await?.is_some() {}
            Ok::<(), Error>(())
        })
        .await
        .map_err(|source| Error::StopTimeout { id, source })?
    }
}

/// An owned reconnect capability for a single existing memory-only terminal.
/// Clone this into a retry task so connection delays do not borrow its SessionStore.
/// All clones still share the same exclusive attachment lock.
#[derive(Clone)]
pub struct LiveAttacher {
    id: u64,
    endpoint: Endpoint,
    lock: PathBuf,
    socket: PathBuf,
}
impl LiveAttacher {
    /// Reattach from an authoritative output offset; never replay input or spawn a process.
    pub async fn attach_from(&self, offset: u64) -> Result<Attached, Error> {
        let journal = journal::Journal::memory_only(&self.lock)?;
        attach_existing(
            self.id,
            self.endpoint.clone(),
            self.socket.clone(),
            journal,
            Vec::new(),
            offset,
        )
        .await
    }
}
async fn attach_existing(
    id: u64,
    endpoint: Endpoint,
    socket: PathBuf,
    journal: journal::Journal,
    history: Vec<Record>,
    offset: u64,
) -> Result<Attached, Error> {
    let tunnel = if matches!(endpoint, Endpoint::Local) {
        None
    } else {
        Some(Arc::new(ssh::tunnel(
            &endpoint,
            &format!("s{id}.sock"),
            &socket,
        )?))
    };
    let mut attempts = 0;
    let attachment = loop {
        match tokio::time::timeout(
            Duration::from_secs(2),
            humata2_holder::attach(&socket, offset),
        )
        .await
        .map_err(|source| Error::Timeout { id, source })?
        {
            Ok(a) => break a,
            Err(_) if tunnel.is_some() && attempts < 50 => {
                attempts += 1;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(source) => {
                return Err(Error::Holder {
                    id,
                    operation: "reattach to",
                    source,
                });
            }
        }
    };
    let Attachment {
        welcome,
        events,
        control,
    } = attachment;
    let journal = Arc::new(Mutex::new(journal));
    Ok(Attached {
        history,
        replay_end: welcome.end,
        cols: welcome.cols,
        rows: welcome.rows,
        exited: welcome.exited,
        reader: Reader {
            id,
            events,
            offset,
            journal: journal.clone(),
            _tunnel: tunnel.clone(),
        },
        writer: Writer {
            id,
            control,
            seq: 0,
            journal,
            _tunnel: tunnel,
        },
    })
}

pub struct Attached {
    pub history: Vec<Record>,
    /// Exclusive output offset captured by the holder's attachment handshake.
    /// Recovered renderers parse this history without repeating PTY replies.
    pub replay_end: u64,
    pub cols: u16,
    pub rows: u16,
    pub exited: Option<i32>,
    pub reader: Reader,
    pub writer: Writer,
}
pub struct Reader {
    id: u64,
    events: Events,
    offset: u64,
    journal: Arc<Mutex<journal::Journal>>,
    _tunnel: Option<Arc<ssh::Tunnel>>,
}
pub struct Writer {
    id: u64,
    control: Control,
    seq: u64,
    journal: Arc<Mutex<journal::Journal>>,
    _tunnel: Option<Arc<ssh::Tunnel>>,
}
#[derive(Debug)]
pub enum Update {
    Record(Record),
    Written(u64),
    Exited(i32),
    Refused(String),
}
impl Reader {
    /// One dedicated task must own this non-cancel-safe stream.
    pub async fn next(&mut self) -> Result<Option<Update>, Error> {
        loop {
            let event = self.events.next().await.map_err(|source| Error::Holder {
                id: self.id,
                operation: "read from",
                source,
            })?;
            let record = match event {
                None => return Ok(None),
                Some(Event::Output { offset, data }) => {
                    if offset > self.offset {
                        return Err(Error::Invalid {
                            reason: format!(
                                "terminal {} output jumped from {} to {offset} without a trim marker; reconnect",
                                self.id, self.offset
                            ),
                        });
                    }
                    let skip = (self.offset - offset).min(data.len() as u64) as usize;
                    if skip == data.len() {
                        continue;
                    }
                    let bytes = data[skip..].to_vec();
                    let start = self.offset;
                    self.offset += bytes.len() as u64;
                    Record::Output {
                        offset: start,
                        bytes,
                    }
                }
                Some(Event::Trimmed { resume_at }) => {
                    let from = self.offset;
                    self.offset = resume_at;
                    Record::Gap {
                        from,
                        to: resume_at,
                    }
                }
                Some(Event::Exited { code }) => return Ok(Some(Update::Exited(code))),
                Some(Event::InputWritten { seq }) => return Ok(Some(Update::Written(seq))),
                Some(Event::Refused { message }) => return Ok(Some(Update::Refused(message))),
            };
            self.journal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .append(&record)?;
            return Ok(Some(Update::Record(record)));
        }
    }
}
impl Writer {
    /// Raw serialized terminal input; acknowledgement arrives as `Update::Written`.
    pub async fn input(&mut self, bytes: &[u8]) -> Result<u64, Error> {
        self.seq += 1;
        self.control
            .input(self.seq, bytes)
            .await
            .map_err(|source| Error::Holder {
                id: self.id,
                operation: "write to",
                source,
            })?;
        Ok(self.seq)
    }
    pub async fn resize(&mut self, cols: u16, rows: u16) -> Result<(), Error> {
        if cols == 0 || rows == 0 || cols > 1000 || rows > 1000 {
            return Err(Error::Invalid {
                reason: "terminal dimensions must be between 1 and 1000 cells".into(),
            });
        }
        self.journal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .append(&Record::Resize { cols, rows })?;
        self.control
            .resize(cols, rows)
            .await
            .map_err(|source| Error::Holder {
                id: self.id,
                operation: "resize",
                source,
            })
    }
    /// Requests shutdown. Keep the Reader/tunnel alive until EOF confirms it, especially over SSH.
    pub async fn stop(&mut self) -> Result<(), Error> {
        self.control
            .shutdown()
            .await
            .map_err(|source| Error::Holder {
                id: self.id,
                operation: "stop",
                source,
            })
    }
}

fn namespace() -> Result<String, Error> {
    use std::io::Read;
    let path = Path::new("/dev/urandom");
    let mut bytes = [0u8; 16];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| disk("allocate a terminal workspace identifier using", path, e))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
