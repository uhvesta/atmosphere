//! Skills sync over the peer link (SK-1, SK-4, SK-9, PER-27, D139).
//!
//! The machine end: [`apply`] takes the host's `ApplySkills` stream (a
//! manifest, then each file in chunks) and hands it to the engine
//! (`humata2_machines::Managed`), which writes beside the live set, checks
//! every hash, and switches with one rename at the end of the stream. A
//! stream that breaks, ends early, or carries a bad file leaves the
//! machine's current set as it was. Then each synced skill is linked into
//! its agent's own folder, `~/.<agent>/skills/<name>`, through the stable
//! `.skills` link, so the agents Humata starts there see it (SK-4, VF-5).
//! Nothing else outside the managed folder is written, and nothing the
//! machine made is replaced or removed.
//!
//! The host end: [`send`] streams a scanned set to a machine over a proven
//! link, reading each file in chunks of at most [`CHUNK_BYTES`] through a
//! small bounded channel, so a large set is never held in memory and the
//! reader waits for the link.
//!
//! The proto's manifest has no set id and no executable bit, so the machine
//! derives both: files under a `bin/` or `scripts/` folder of a skill are
//! executable, the rest are not, and the set id is computed from the
//! entries the way the engine does. A `#!` first line can't be seen before
//! the file arrives, so it doesn't count. The host keeps its own manifest
//! (with the real bits) for planning.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::pb;
use humata2_machines::{Managed, SkillEntry, SkillSet, SkillsError, SkillsManifest, SkillsSource};
use pb::peer_service_client::PeerServiceClient;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
use tonic::transport::Channel;
use tonic::{Code, Status};

use crate::ports as status;

/// The most bytes of one file a single message carries.
pub const CHUNK_BYTES: usize = 1 << 20;

/// Messages the sender may have queued ahead of the link.
const QUEUED_MESSAGES: usize = 4;

/// The manifest format the engine reads (`SkillsManifest::version`).
const ENGINE_FORMAT: u32 = 1;

/// Each agent's skill folder under a home folder, as (agent, folder). One
/// place to add an agent. A folder that isn't there is an agent that isn't
/// installed.
pub const AGENT_FOLDERS: [(&str, &str); 4] = [
    ("agy", ".gemini/antigravity-cli/skills"),
    ("claude", ".claude/skills"),
    ("codex", ".codex/skills"),
    ("copilot", ".copilot/skills"),
];

/// The skill folders of the agents installed under `home`.
pub fn agent_sources(home: &Path) -> Vec<SkillsSource> {
    AGENT_FOLDERS
        .iter()
        .map(|(agent, folder)| SkillsSource {
            agent: (*agent).to_owned(),
            root: home.join(folder),
        })
        .filter(|source| source.root.is_dir())
        .collect()
}

/// Installs the skills Humata ships (D790) for every agent set up under
/// `home`, at the daemon's start. An agent is set up when its own folder
/// (`~/.claude`, `~/.codex`, `~/.copilot`) is there: Humata never makes
/// that folder, so it sets no agent up. A failure is logged and the daemon
/// starts all the same; the next start installs again.
pub fn install_own(home: &Path) {
    for (agent, folder) in AGENT_FOLDERS {
        let root = home.join(folder);
        if !root.parent().is_some_and(Path::is_dir) {
            continue;
        }
        match humata2_machines::install_own_skills(&root) {
            Ok(outcomes) => {
                for kept in outcomes
                    .iter()
                    .filter(|outcome| outcome.change == humata2_machines::OwnSkillChange::Kept)
                {
                    humata2_rpc_kit::log::say!(
                        "humatad2: left {} as it is: it is your own skill, not the `{}` skill Humata ships, so {agent} doesn't have Humata's. To get Humata's, rename or delete yours; it is installed when the daemon next starts",
                        root.join(&kept.skill).display(),
                        kept.skill
                    );
                }
            }
            Err(error) => humata2_rpc_kit::log::say!(
                "humatad2: couldn't install Humata's skills for {agent} in {}: {}. Agents still run, without those skills. Check that the folder is yours and writable and the disk isn't full; they are installed when the daemon next starts",
                root.display(),
                status::chain(&error)
            ),
        }
    }
}

/// Where a machine keeps the synced set, and its own skills (for SK-5).
#[derive(Clone)]
pub struct MachineSkills {
    managed_root: PathBuf,
    locals: Vec<SkillsSource>,
}

impl MachineSkills {
    /// The machine's Humata folder is `<home>/Humata` (D140); its own
    /// skills are in each agent's folder under `home`. The local folders
    /// are looked up at each apply, so an agent installed later counts.
    pub fn under_home(home: &Path) -> Self {
        Self {
            managed_root: home.join("Humata"),
            locals: AGENT_FOLDERS
                .iter()
                .map(|(agent, folder)| SkillsSource {
                    agent: (*agent).to_owned(),
                    root: home.join(folder),
                })
                .collect(),
        }
    }

    /// As [`MachineSkills::under_home`] for `$HOME`; none when it is unset.
    pub fn from_env() -> Option<Self> {
        std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(|home| Self::under_home(Path::new(&home)))
    }
}

/// What `ApplySkills` hands to PeerService: `skills` is none when the
/// machine has no home folder to put a set in.
pub async fn handle(
    skills: Option<&MachineSkills>,
    stream: tonic::Streaming<pb::SkillsMessage>,
) -> Result<pb::ApplySkillsResponse, Status> {
    let Some(skills) = skills else {
        return Err(status_of(&ApplyError::NoHome));
    };
    apply(&skills.managed_root, &skills.locals, stream).await
}

/// Why a set couldn't be applied on the machine.
#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("the stream ended before a manifest came")]
    NoManifest,
    #[error("the first message of the stream wasn't the manifest")]
    NotManifestFirst,
    #[error("the stream carried a second manifest")]
    SecondManifest,
    #[error("the stream carried an empty message")]
    EmptyMessage,
    #[error("the host's stream broke before it was complete: {reason}")]
    Stream {
        reason: String,
        #[source]
        source: Status,
    },
    #[error("{source}")]
    Engine {
        #[source]
        source: SkillsError,
    },
    #[error("this machine has no HOME to keep the Humata folder in")]
    NoHome,
    #[error("the new skills are on this machine, but linking them into the agents' folders failed: {source}")]
    Link {
        #[source]
        source: SkillsError,
    },
    #[error("applying the set stopped before it finished: {source}")]
    Task {
        #[source]
        source: tokio::task::JoinError,
    },
}

impl ApplyError {
    fn fix(&self) -> &'static str {
        match self {
            Self::Engine { .. } => {
                "the machine's current skills were kept; fix the cause above and sync again"
            }
            Self::NoHome => "start humatad2 on this machine with HOME set, then sync again",
            Self::Link { .. } => {
                "the machine's agents don't see the new skills yet; fix the cause above and sync again"
            }
            _ => "the machine's current skills were kept; sync again from the host (update Humata on both sides if it keeps failing)",
        }
    }
}

impl From<SkillsError> for ApplyError {
    fn from(source: SkillsError) -> Self {
        Self::Engine { source }
    }
}

fn status_of(error: &ApplyError) -> Status {
    let code = match error {
        ApplyError::Engine { source } => match source {
            SkillsError::Io { .. } => Code::Internal,
            SkillsError::ForeignSkillsPath { .. } => Code::FailedPrecondition,
            SkillsError::HashMismatch { .. } | SkillsError::Incomplete { .. } => Code::DataLoss,
            _ => Code::InvalidArgument,
        },
        ApplyError::Stream { .. } => Code::Aborted,
        ApplyError::NoHome => Code::FailedPrecondition,
        ApplyError::Link { .. } | ApplyError::Task { .. } => Code::Internal,
        _ => Code::InvalidArgument,
    };
    status::failure(
        code,
        pb::Error {
            message: status::chain(error),
            fix: error.fix().to_owned(),
            retry: None,
            kind: None,
        },
    )
}

/// Applies the set `stream` carries into the managed folder at
/// `managed_root`, switching to it only when the whole stream arrived and
/// every file checks, then links the set into `locals`, each agent's own
/// skill folder (VF-5). The response names the synced skills whose name the
/// machine's own skills also have there (SK-5), as `agent/skill`; Humata's
/// links don't count.
pub async fn apply<S>(
    managed_root: &Path,
    locals: &[SkillsSource],
    mut stream: S,
) -> Result<pb::ApplySkillsResponse, Status>
where
    S: Stream<Item = Result<pb::SkillsMessage, Status>> + Unpin,
{
    apply_inner(managed_root, locals, &mut stream)
        .await
        .map_err(|error| status_of(&error))
}

async fn apply_inner<S>(
    managed_root: &Path,
    locals: &[SkillsSource],
    stream: &mut S,
) -> Result<pb::ApplySkillsResponse, ApplyError>
where
    S: Stream<Item = Result<pb::SkillsMessage, Status>> + Unpin,
{
    let manifest = match stream.next().await {
        Some(Ok(pb::SkillsMessage {
            message: Some(pb::skills_message::Message::Manifest(manifest)),
        })) => manifest,
        Some(Ok(_)) => return Err(ApplyError::NotManifestFirst),
        Some(Err(source)) => return Err(stream_broke(source)),
        None => return Err(ApplyError::NoManifest),
    };
    let version = manifest.version;
    let engine = engine_manifest(&manifest)?;
    let skills = skill_names(&engine);
    let managed = Managed::open(managed_root)?;
    // Dropping `incoming` on any error below removes the partial files and
    // leaves the current set alone.
    let mut incoming = managed.begin(engine)?;
    while let Some(next) = stream.next().await {
        let message = next.map_err(stream_broke)?;
        match message.message {
            Some(pb::skills_message::Message::File(file)) => {
                let path = joined(&file.agent, &file.path)?;
                incoming.write(&path, file.offset, &file.data)?;
            }
            Some(pb::skills_message::Message::Manifest(_)) => return Err(ApplyError::SecondManifest),
            None => return Err(ApplyError::EmptyMessage),
        }
    }
    let agents = locals.to_vec();
    let clashes = tokio::task::spawn_blocking(move || {
        incoming.finish()?;
        managed
            .link_agents(&agents)
            .map_err(|source| ApplyError::Link { source })?;
        Ok::<_, ApplyError>(clashes(&skills, &agents, &managed))
    })
    .await
    .map_err(|source| ApplyError::Task { source })??;
    Ok(pb::ApplySkillsResponse { version, clashes })
}

fn stream_broke(source: Status) -> ApplyError {
    ApplyError::Stream {
        reason: source.message().to_owned(),
        source,
    }
}

fn joined(agent: &str, path: &str) -> Result<String, SkillsError> {
    if agent.is_empty() || agent.contains(['/', '\\', '\0']) {
        return Err(SkillsError::BadAgent {
            agent: agent.to_owned(),
        });
    }
    Ok(format!("{agent}/{path}"))
}

/// The engine's manifest for the proto's: paths joined, the executable bit
/// and set id derived (see the module doc).
fn engine_manifest(manifest: &pb::SkillsManifest) -> Result<SkillsManifest, SkillsError> {
    let mut entries = Vec::with_capacity(manifest.entries.len());
    for entry in &manifest.entries {
        let path = joined(&entry.agent, &entry.path)?;
        entries.push(SkillEntry {
            executable: looks_executable(&path),
            path,
            sha256: entry.sha256.clone(),
            size: entry.size,
            source: PathBuf::new(),
        });
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries.dedup_by(|a, b| a.path == b.path);
    Ok(SkillsManifest {
        version: ENGINE_FORMAT,
        set_id: set_id(&entries),
        entries,
    })
}

/// Under a `bin` or `scripts` folder inside a skill.
fn looks_executable(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    parts.len() > 3 && parts[2..parts.len() - 1].iter().any(|dir| *dir == "bin" || *dir == "scripts")
}

/// The engine's set id: SHA-256 over each sorted entry's path, hash and
/// mode. Kept in step with `humata2_machines`'s own (a test compares them).
fn set_id(sorted: &[SkillEntry]) -> String {
    let mut hasher = Sha256::new();
    for entry in sorted {
        hasher.update(entry.path.as_bytes());
        hasher.update([0]);
        hasher.update(entry.sha256.as_bytes());
        hasher.update([0]);
        hasher.update(if entry.executable { b"x" } else { b"-" });
        hasher.update(b"\n");
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Each (agent, skill) the manifest lists.
fn skill_names(manifest: &SkillsManifest) -> BTreeSet<(String, String)> {
    manifest
        .entries
        .iter()
        .filter_map(|entry| {
            let mut parts = entry.path.splitn(3, '/');
            Some((parts.next()?.to_owned(), parts.next()?.to_owned()))
        })
        .collect()
}

/// The synced skills a machine-made skill has the name of, as `agent/skill`.
/// Humata's own links in the agents' folders aren't the machine's skills.
fn clashes(skills: &BTreeSet<(String, String)>, locals: &[SkillsSource], managed: &Managed) -> Vec<String> {
    skills
        .iter()
        .filter(|(agent, skill)| {
            locals.iter().filter(|local| local.agent == *agent).any(|local| {
                let slot = local.root.join(skill);
                std::fs::symlink_metadata(&slot).is_ok() && !managed.owns_link(&slot)
            })
        })
        .map(|(agent, skill)| format!("{agent}/{skill}"))
        .collect()
}

/// Why a set couldn't be sent.
#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("couldn't read {} to send it: {source}", .path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{} changed while it was being sent; sync again once your edit is saved", .path.display())]
    Changed { path: PathBuf },
    #[error("the machine didn't take the skills: {reason}")]
    Machine {
        reason: String,
        #[source]
        source: Status,
    },
    #[error("the machine answered for set version {got}, not the {sent} that was sent; sync again")]
    WrongAnswer { sent: u64, got: u64 },
    #[error("sending the skills stopped before it finished: {source}")]
    Task {
        #[source]
        source: tokio::task::JoinError,
    },
}

impl SendError {
    /// What to do about it, for the person.
    pub fn fix(&self) -> &'static str {
        match self {
            Self::Read { .. } => "make the file readable on this Mac or exclude it in Settings, then sync again",
            Self::Changed { .. } => "sync again",
            Self::Machine { .. } => "the machine's skills were kept as they were; the reason above says what to fix, then sync again",
            Self::WrongAnswer { .. } | Self::Task { .. } => "sync again; if it keeps failing, update Humata on both sides",
        }
    }
}

/// The `version` a set carries on the link: the first 64 bits of its id.
pub fn version_of(set_id: &str) -> u64 {
    set_id
        .get(..16)
        .and_then(|prefix| u64::from_str_radix(prefix, 16).ok())
        .unwrap_or(0)
}

/// Sends `set` over the proven link behind `peer` and answers with what the
/// machine said.
pub async fn send(
    peer: &mut PeerServiceClient<Channel>,
    set: &SkillSet,
) -> Result<pb::ApplySkillsResponse, SendError> {
    let version = version_of(set.set_id());
    let entries = set.entries().to_vec();
    let (queue, messages) = mpsc::channel(QUEUED_MESSAGES);
    let producer = tokio::task::spawn_blocking(move || produce(&entries, version, &queue));
    let answer = peer.apply_skills(ReceiverStream::new(messages)).await;
    match producer.await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return Err(error),
        Err(source) => return Err(SendError::Task { source }),
    }
    let response = answer
        .map_err(|source| SendError::Machine {
            reason: source.message().to_owned(),
            source,
        })?
        .into_inner();
    if response.version != version {
        return Err(SendError::WrongAnswer {
            sent: version,
            got: response.version,
        });
    }
    Ok(response)
}

/// The manifest, then each file in chunks, onto `queue` as the link takes
/// them. Blocking: reads files and waits for room. Stops quietly when the
/// receiving side is gone (the call already failed).
fn produce(
    entries: &[SkillEntry],
    version: u64,
    queue: &mpsc::Sender<pb::SkillsMessage>,
) -> Result<(), SendError> {
    use std::io::Read;
    let listed = entries
        .iter()
        .map(|entry| {
            let (agent, path) = entry.path.split_once('/').unwrap_or((entry.path.as_str(), ""));
            pb::SkillsManifestEntry {
                agent: agent.to_owned(),
                path: path.to_owned(),
                sha256: entry.sha256.clone(),
                size: entry.size,
            }
        })
        .collect();
    let manifest = pb::SkillsMessage {
        message: Some(pb::skills_message::Message::Manifest(pb::SkillsManifest {
            version,
            entries: listed,
        })),
    };
    if queue.blocking_send(manifest).is_err() {
        return Ok(());
    }
    let mut buffer = vec![0u8; CHUNK_BYTES];
    for entry in entries {
        let read_failed = |source: std::io::Error| SendError::Read {
            path: entry.source.clone(),
            source,
        };
        let (agent, path) = entry.path.split_once('/').unwrap_or((entry.path.as_str(), ""));
        let mut file = std::fs::File::open(&entry.source).map_err(read_failed)?;
        let mut offset = 0u64;
        loop {
            let mut filled = 0;
            while filled < buffer.len() {
                let n = file.read(&mut buffer[filled..]).map_err(read_failed)?;
                if n == 0 {
                    break;
                }
                filled += n;
            }
            let last = filled < buffer.len() || offset + filled as u64 >= entry.size;
            if offset + filled as u64 > entry.size {
                return Err(SendError::Changed {
                    path: entry.source.clone(),
                });
            }
            let chunk = pb::SkillsMessage {
                message: Some(pb::skills_message::Message::File(pb::SkillsFile {
                    agent: agent.to_owned(),
                    path: path.to_owned(),
                    offset,
                    data: buffer[..filled].to_vec(),
                    last,
                })),
            };
            offset += filled as u64;
            if queue.blocking_send(chunk).is_err() {
                return Ok(());
            }
            if last {
                break;
            }
        }
        if offset != entry.size {
            return Err(SendError::Changed {
                path: entry.source.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use humata2_machines::scan_skills;

    use super::*;

    fn write(root: &Path, relative: &str, bytes: &[u8]) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// A host home with two skills, one of them with a file over a chunk.
    fn host() -> (tempfile::TempDir, SkillSet) {
        let dir = tempfile::tempdir().unwrap();
        let skills = dir.path().join(".claude/skills");
        write(&skills, "alpha/SKILL.md", b"alpha\n");
        write(&skills, "alpha/notes/more.md", b"more\n");
        write(&skills, "beta/SKILL.md", b"");
        write(&skills, "beta/big.bin", &vec![7u8; CHUNK_BYTES * 2 + 10]);
        let set = scan_skills(&agent_sources(dir.path()), &[]).unwrap();
        (dir, set)
    }

    fn messages(set: &SkillSet) -> Vec<pb::SkillsMessage> {
        let (queue, mut messages) = mpsc::channel(10_000);
        // `produce` waits for room with a blocking send, which a thread
        // inside the async runtime may not do; `send` runs it on the
        // blocking pool, the tests on a thread of their own.
        std::thread::scope(|scope| {
            scope
                .spawn(|| produce(set.entries(), version_of(set.set_id()), &queue))
                .join()
                .unwrap()
                .unwrap();
        });
        drop(queue);
        let mut all = Vec::new();
        while let Ok(message) = messages.try_recv() {
            all.push(message);
        }
        all
    }

    fn stream(messages: Vec<pb::SkillsMessage>) -> impl Stream<Item = Result<pb::SkillsMessage, Status>> + Unpin {
        tokio_stream::iter(messages.into_iter().map(Ok))
    }

    fn files(messages: &[pb::SkillsMessage]) -> Vec<&pb::SkillsFile> {
        messages
            .iter()
            .filter_map(|message| match &message.message {
                Some(pb::skills_message::Message::File(file)) => Some(file),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_set_is_sent_as_a_manifest_then_chunks_no_larger_than_the_limit() {
        let (_dir, set) = host();
        let all = messages(&set);
        assert!(matches!(
            all[0].message,
            Some(pb::skills_message::Message::Manifest(_))
        ));
        let chunks = files(&all);
        assert!(chunks.iter().all(|chunk| chunk.data.len() <= CHUNK_BYTES));
        let big: Vec<_> = chunks.iter().filter(|chunk| chunk.path == "beta/big.bin").collect();
        assert_eq!(big.len(), 3);
        assert_eq!(big.iter().map(|chunk| chunk.data.len()).sum::<usize>(), CHUNK_BYTES * 2 + 10);
        assert_eq!(big.iter().filter(|chunk| chunk.last).count(), 1);
        assert!(big.last().unwrap().last);
        assert_eq!(big[1].offset, CHUNK_BYTES as u64);
        // The empty file still gets its one closing chunk.
        assert!(chunks.iter().any(|chunk| chunk.agent == "claude" && chunk.path == "beta/SKILL.md" && chunk.last));
    }

    #[tokio::test]
    async fn a_sent_set_becomes_the_machines_current_set_and_clashes_are_named() {
        let (_dir, set) = host();
        let machine = tempfile::tempdir().unwrap();
        let root = machine.path().join("Humata");
        let local = machine.path().join(".claude/skills");
        std::fs::create_dir_all(local.join("alpha")).unwrap();
        let locals = [SkillsSource { agent: "claude".to_owned(), root: local }];

        let response = apply(&root, &locals, stream(messages(&set))).await.unwrap();
        assert_eq!(response.version, version_of(set.set_id()));
        assert_eq!(response.clashes, vec!["claude/alpha".to_owned()]);

        let live = Managed::open(&root).unwrap().current_manifest().unwrap().unwrap();
        assert_eq!(live.entries.len(), set.entries().len());
        assert_eq!(
            std::fs::read(root.join(".skills/claude/alpha/SKILL.md")).unwrap(),
            b"alpha\n"
        );
        assert_eq!(
            std::fs::metadata(root.join(".skills/claude/beta/big.bin")).unwrap().len(),
            (CHUNK_BYTES * 2 + 10) as u64
        );
    }

    #[test]
    fn the_machines_set_id_is_the_engines_for_the_same_files() {
        let (_dir, set) = host();
        let sent = match &messages(&set)[0].message {
            Some(pb::skills_message::Message::Manifest(manifest)) => manifest.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(engine_manifest(&sent).unwrap().set_id, set.set_id());
    }

    #[test]
    fn only_files_under_bin_or_scripts_in_a_skill_are_executable() {
        assert!(looks_executable("claude/alpha/bin/run"));
        assert!(looks_executable("claude/alpha/x/scripts/run.sh"));
        assert!(!looks_executable("claude/alpha/SKILL.md"));
        assert!(!looks_executable("claude/bin/tool"));
        assert!(!looks_executable("claude/alpha/bin"));
    }

    #[tokio::test]
    async fn a_stream_that_ends_early_keeps_the_machines_old_set() {
        let (_dir, set) = host();
        let machine = tempfile::tempdir().unwrap();
        let root = machine.path().join("Humata");
        apply(&root, &[], stream(messages(&set))).await.unwrap();
        let before = Managed::open(&root).unwrap().current_manifest().unwrap().unwrap();

        let smaller = tempfile::tempdir().unwrap();
        write(&smaller.path().join(".claude/skills"), "gamma/SKILL.md", b"gamma");
        write(&smaller.path().join(".claude/skills"), "gamma/other.md", b"other");
        let next = scan_skills(&agent_sources(smaller.path()), &[]).unwrap();
        let mut cut = messages(&next);
        cut.pop();
        let refused = apply(&root, &[], stream(cut)).await.unwrap_err();
        assert_ne!(refused.code(), Code::Ok);

        let after = Managed::open(&root).unwrap().current_manifest().unwrap().unwrap();
        assert_eq!(after.set_id, before.set_id);
        assert!(root.join(".skills/claude/alpha/SKILL.md").exists());
        assert!(!root.join(".skills/claude/gamma").exists());
    }

    #[tokio::test]
    async fn a_stream_that_breaks_keeps_the_machines_old_set() {
        let (_dir, set) = host();
        let machine = tempfile::tempdir().unwrap();
        let root = machine.path().join("Humata");
        apply(&root, &[], stream(messages(&set))).await.unwrap();
        let mut broken: Vec<Result<pb::SkillsMessage, Status>> =
            messages(&set).into_iter().take(2).map(Ok).collect();
        broken.push(Err(Status::unavailable("link lost")));
        let refused = apply(&root, &[], tokio_stream::iter(broken)).await.unwrap_err();
        assert_eq!(refused.code(), Code::Aborted);
        assert!(root.join(".skills/claude/alpha/SKILL.md").exists());
    }

    #[tokio::test]
    async fn a_stream_whose_first_message_isnt_the_manifest_is_refused() {
        let (_dir, set) = host();
        let machine = tempfile::tempdir().unwrap();
        let root = machine.path().join("Humata");
        let mut all = messages(&set);
        all.swap(0, 1);
        let refused = apply(&root, &[], stream(all)).await.unwrap_err();
        assert_eq!(refused.code(), Code::InvalidArgument);
        assert!(!root.join(".skills").exists());
    }

    #[tokio::test]
    async fn a_second_manifest_is_refused() {
        let (_dir, set) = host();
        let machine = tempfile::tempdir().unwrap();
        let root = machine.path().join("Humata");
        let mut all = messages(&set);
        all.insert(1, all[0].clone());
        let refused = apply(&root, &[], stream(all)).await.unwrap_err();
        assert_eq!(refused.code(), Code::InvalidArgument);
        assert!(!root.join(".skills").exists());
    }

    #[tokio::test]
    async fn an_unsafe_path_is_refused_and_nothing_is_written() {
        let machine = tempfile::tempdir().unwrap();
        let root = machine.path().join("Humata");
        let evil = pb::SkillsMessage {
            message: Some(pb::skills_message::Message::Manifest(pb::SkillsManifest {
                version: 1,
                entries: vec![pb::SkillsManifestEntry {
                    agent: "claude".to_owned(),
                    path: "../../escape".to_owned(),
                    sha256: "00".repeat(32),
                    size: 1,
                }],
            })),
        };
        let refused = apply(&root, &[], stream(vec![evil])).await.unwrap_err();
        assert_eq!(refused.code(), Code::InvalidArgument);
        assert!(!machine.path().join("escape").exists());
        assert!(!root.join(".skills").exists());
    }

    #[tokio::test]
    async fn a_wrong_hash_is_refused_and_the_old_set_stays() {
        let (_dir, set) = host();
        let machine = tempfile::tempdir().unwrap();
        let root = machine.path().join("Humata");
        apply(&root, &[], stream(messages(&set))).await.unwrap();

        let other = tempfile::tempdir().unwrap();
        write(&other.path().join(".claude/skills"), "gamma/SKILL.md", b"gamma");
        let next = scan_skills(&agent_sources(other.path()), &[]).unwrap();
        let mut all = messages(&next);
        if let Some(pb::skills_message::Message::File(file)) = &mut all[1].message {
            file.data = b"GAMMA".to_vec();
        }
        let refused = apply(&root, &[], stream(all)).await.unwrap_err();
        assert_eq!(refused.code(), Code::DataLoss);
        assert!(root.join(".skills/claude/alpha/SKILL.md").exists());
    }

    #[tokio::test]
    async fn a_real_skills_folder_is_refused_and_left_alone() {
        let (_dir, set) = host();
        let machine = tempfile::tempdir().unwrap();
        let root = machine.path().join("Humata");
        write(&root, ".skills/mine/SKILL.md", b"mine");
        let refused = apply(&root, &[], stream(messages(&set))).await.unwrap_err();
        assert_eq!(refused.code(), Code::FailedPrecondition);
        assert_eq!(std::fs::read(root.join(".skills/mine/SKILL.md")).unwrap(), b"mine");
    }

    /// A host home whose Copilot has the skills named, each with a
    /// SKILL.md saying `<name> from the host`.
    fn copilot_set(skills: &[&str]) -> (tempfile::TempDir, SkillSet) {
        let dir = tempfile::tempdir().unwrap();
        for skill in skills {
            write(
                &dir.path().join(".copilot/skills"),
                &format!("{skill}/SKILL.md"),
                format!("{skill} from the host").as_bytes(),
            );
        }
        let set = scan_skills(&agent_sources(dir.path()), &[]).unwrap();
        (dir, set)
    }

    /// A machine home with a Copilot skill of its own, `mine`.
    fn machine_with_mine() -> (tempfile::TempDir, MachineSkills) {
        let home = tempfile::tempdir().unwrap();
        write(&home.path().join(".copilot/skills"), "mine/SKILL.md", b"made on the machine");
        let skills = MachineSkills::under_home(home.path());
        (home, skills)
    }

    async fn sync(machine: &MachineSkills, set: &SkillSet) -> pb::ApplySkillsResponse {
        apply(&machine.managed_root, &machine.locals, stream(messages(set))).await.unwrap()
    }

    fn is_link(path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
    }

    #[tokio::test]
    async fn a_synced_skill_shows_in_the_agents_own_folder_and_the_machines_own_is_untouched() {
        let (home, machine) = machine_with_mine();
        let (_host, set) = copilot_set(&["a", "mine"]);

        let answer = sync(&machine, &set).await;

        let folder = home.path().join(".copilot/skills");
        let a = folder.join("a");
        assert!(is_link(&a), "the agent's folder lists the synced skill `a`");
        assert_eq!(
            std::fs::read_link(&a).unwrap(),
            home.path().join("Humata/.skills/copilot/a"),
            "it resolves into the set through the stable `.skills` link"
        );
        assert_eq!(std::fs::read(a.join("SKILL.md")).unwrap(), b"a from the host");
        let mine = folder.join("mine");
        assert!(!is_link(&mine) && mine.is_dir());
        assert_eq!(std::fs::read(mine.join("SKILL.md")).unwrap(), b"made on the machine");
        assert_eq!(answer.clashes, vec!["copilot/mine".to_owned()]);

        // Syncing the same set again finds Humata's own link no clash.
        assert_eq!(sync(&machine, &set).await.clashes, vec!["copilot/mine".to_owned()]);
    }

    #[tokio::test]
    async fn a_later_sync_without_a_skill_removes_only_its_link() {
        let (home, machine) = machine_with_mine();
        let (_first, set) = copilot_set(&["a", "b", "mine"]);
        sync(&machine, &set).await;
        let (_second, smaller) = copilot_set(&["b", "mine"]);

        sync(&machine, &smaller).await;

        let folder = home.path().join(".copilot/skills");
        assert!(std::fs::symlink_metadata(folder.join("a")).is_err(), "the link to `a` is gone");
        assert!(is_link(&folder.join("b")));
        assert_eq!(std::fs::read(folder.join("b/SKILL.md")).unwrap(), b"b from the host");
        assert_eq!(std::fs::read(folder.join("mine/SKILL.md")).unwrap(), b"made on the machine");
    }

    #[tokio::test]
    async fn a_real_folder_or_someone_elses_link_named_like_a_synced_skill_is_never_replaced() {
        let (home, machine) = machine_with_mine();
        let folder = home.path().join(".copilot/skills");
        let elsewhere = tempfile::tempdir().unwrap();
        write(elsewhere.path(), "SKILL.md", b"the user's own link");
        std::os::unix::fs::symlink(elsewhere.path(), folder.join("theirs")).unwrap();
        let (_host, set) = copilot_set(&["a", "mine", "theirs"]);

        let answer = sync(&machine, &set).await;
        assert_eq!(answer.clashes, vec!["copilot/mine".to_owned(), "copilot/theirs".to_owned()]);
        // A later sync that drops them doesn't take them away either.
        let (_later, later) = copilot_set(&["a"]);
        sync(&machine, &later).await;

        assert!(!is_link(&folder.join("mine")));
        assert_eq!(std::fs::read(folder.join("mine/SKILL.md")).unwrap(), b"made on the machine");
        assert_eq!(std::fs::read_link(folder.join("theirs")).unwrap(), elsewhere.path());
        assert_eq!(std::fs::read(folder.join("theirs/SKILL.md")).unwrap(), b"the user's own link");
        assert!(is_link(&folder.join("a")));
    }

    #[tokio::test]
    async fn an_agent_with_no_folder_yet_gets_one_holding_the_links() {
        let home = tempfile::tempdir().unwrap();
        let machine = MachineSkills::under_home(home.path());
        let (_host, set) = copilot_set(&["a"]);

        sync(&machine, &set).await;

        let a = home.path().join(".copilot/skills/a");
        assert!(is_link(&a));
        assert_eq!(std::fs::read(a.join("SKILL.md")).unwrap(), b"a from the host");
        assert!(!home.path().join(".claude").exists(), "an agent with no synced skills gets no folder");
    }
}
