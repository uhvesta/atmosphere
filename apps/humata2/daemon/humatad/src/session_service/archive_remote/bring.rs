//! Bringing over what a machine's copy of a restored session had more of
//! (D224). The host copies the machine's staged records and conversation,
//! and its session folder if it still has one, into a side folder of the
//! restored session: `.humata/from-<machine>-<operation>/`. The copy is
//! built as `<side>.partial` and renamed only once it is checked and its
//! notice written, so a folder without the `.partial` ending is whole.
//! Only then does the machine's copy go.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{Brought, CarryError, check_brought};
use tonic::{Code, Status};

use super::compare::{Extra, More};
use super::{Carry, Place, SessionSvc, blocking, now_ms};

/// The notice inside a side folder, which marks the carry as done.
const NOTICE: &str = "NOTICE.md";

/// Why a machine's copy couldn't be brought over. The copy stays on the
/// machine and Humata tries again at the next link-up.
#[derive(Debug, thiserror::Error)]
pub(super) enum BringError {
    #[error("the session's record here couldn't be read: {source}; Humata tries again when the machine is next linked or Humata next starts")]
    Record {
        #[source]
        source: Status,
    },
    #[error("couldn't {doing} on {machine}: {source}; its copy stays there and Humata tries again when it is next linked")]
    Call {
        machine: String,
        doing: &'static str,
        #[source]
        source: Status,
    },
    #[error("couldn't {doing} {}: {source}; free disk space on this Mac or check that the session's folder is yours, then Humata tries again at the next link-up", path.display())]
    Disk {
        doing: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("what arrived from {machine} isn't what it sent: {source}; its copy stays there and Humata copies it again at the next link-up")]
    Check {
        machine: String,
        #[source]
        source: CarryError,
    },
    #[error("the session's folder {} has no name Humata can keep a side folder under; its copy stays on the machine", path.display())]
    NoFolder { path: PathBuf },
    #[error("a background task failed: {source}; Humata tries again at the next link-up")]
    Task {
        #[source]
        source: Status,
    },
}

impl BringError {
    /// The link to the machine failed: nothing to report, the next link-up
    /// tries again.
    pub(super) fn is_link(&self) -> bool {
        matches!(
            self,
            Self::Call { source, .. } | Self::Record { source }
                if matches!(source.code(), Code::Unavailable | Code::Unknown | Code::Cancelled)
        )
    }
}

fn flat<T>(done: Result<Result<T, BringError>, Status>) -> Result<T, BringError> {
    done.map_err(|source| BringError::Task { source })?
}

fn disk(doing: &'static str, path: &Path) -> impl FnOnce(std::io::Error) -> BringError {
    let path = path.to_owned();
    move |source| BringError::Disk { doing, path, source }
}

/// Removes `path` and what is under it; a path that isn't there is fine.
fn clear(path: &Path) -> Result<(), BringError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(BringError::Disk { doing: "remove", path: path.to_owned(), source }),
    }
}

/// What a carry is for.
pub(super) struct Bringing<'a> {
    pub(super) machine: &'a str,
    /// The restore's operation, which names the side folder.
    pub(super) operation: &'a str,
    pub(super) session: &'a str,
    pub(super) more: &'a More,
    /// The machine's own archive summary, when the comparison got it.
    pub(super) summary: Option<&'a pb::ArchiveSummary>,
}

impl SessionSvc {
    /// Carries what the machine holds of the session into the side folder
    /// and returns it. Done already (a side folder with its notice) counts
    /// as done. A failure leaves no partial folder here and the machine's
    /// staged files unstaged; the machine's copy is untouched either way.
    pub(super) async fn bring_over(&self, carry: &Carry, bringing: Bringing<'_>) -> Result<PathBuf, BringError> {
        let loaded = self.load(bringing.session).await.map_err(|source| BringError::Record { source })?;
        let record = loaded.record;
        let Some(name) = record.folder.file_name().map(|name| name.to_string_lossy().into_owned()) else {
            return Err(BringError::NoFolder { path: record.folder });
        };
        let sides = record.folder.join(".humata");
        let label = format!("from-{}-{}", bringing.machine, bringing.operation);
        let (done, partial) = (sides.join(&label), sides.join(format!("{label}.partial")));

        let settled = self.bring_into(carry, &bringing, &record.channel, &name, &label, &done, &partial).await;
        if settled.is_err() {
            let partial = partial.clone();
            let _ = blocking(move || clear(&partial)).await;
            if let Ok(channel) = self.find_channel(&record.channel).await {
                let from = Place {
                    machine: bringing.machine.to_owned(),
                    channel: channel.id.clone(),
                    folder: channel.folder.clone(),
                };
                let _ = carry
                    .abort(bringing.machine, bringing.operation, bringing.session, &from, "", false)
                    .await;
            }
        }
        settled
    }

    #[allow(clippy::too_many_arguments)]
    async fn bring_into(
        &self,
        carry: &Carry,
        bringing: &Bringing<'_>,
        channel_id: &str,
        name: &str,
        label: &str,
        done: &Path,
        partial: &Path,
    ) -> Result<PathBuf, BringError> {
        let call = |doing: &'static str| {
            let machine = bringing.machine.to_owned();
            move |source: Status| BringError::Call { machine, doing, source }
        };
        let finished = {
            let (done, partial) = (done.to_owned(), partial.to_owned());
            flat(
                blocking(move || -> Result<bool, BringError> {
                    clear(&partial)?;
                    if done.join(NOTICE).is_file() {
                        return Ok(true);
                    }
                    clear(&done)?;
                    Ok(false)
                })
                .await,
            )?
        };
        if finished {
            return Ok(done.to_owned());
        }

        let channel = self.find_channel(channel_id).await.map_err(|source| BringError::Record { source })?;
        let at = |machine: &str| Place {
            machine: machine.to_owned(),
            channel: channel.id.clone(),
            folder: channel.folder.clone(),
        };
        let (from, here) = (at(bringing.machine), at(&self.machine));
        let packed = carry
            .pack(bringing.machine, bringing.operation, bringing.session)
            .await
            .map_err(call("stage the session's copy"))?;
        // The copy lands each item under its own name in the partial folder.
        let into = format!("sessions/{name}/.humata/{label}.partial");
        let stamp = now_ms();
        carry
            .copy(
                &format!("{}_bring_stage_{stamp}", bringing.operation),
                (&from, &packed.stage),
                (&here, &into),
            )
            .await
            .map_err(call("copy the session's staged files"))?;
        let has_folder = carry.exists(&from, &packed.folder).await.map_err(call("look for the session's folder"))?;
        if has_folder {
            carry
                .copy(
                    &format!("{}_bring_folder_{stamp}", bringing.operation),
                    (&from, &packed.folder),
                    (&here, &into),
                )
                .await
                .map_err(call("copy the session's folder"))?;
        }

        let (folder_name, machine) = (
            Path::new(&packed.folder).file_name().map(|name| name.to_string_lossy().into_owned()),
            bringing.machine.to_owned(),
        );
        let (done_path, partial_path) = (done.to_owned(), partial.to_owned());
        let facts = Facts {
            machine: bringing.machine.to_owned(),
            operation: bringing.operation.to_owned(),
            session: bringing.session.to_owned(),
            more: bringing.more.clone(),
            summary: bringing.summary.cloned(),
        };
        flat(
            blocking(move || -> Result<(), BringError> {
                if has_folder && let Some(copied) = folder_name {
                    let (from, to) = (partial_path.join(copied), partial_path.join("folder"));
                    std::fs::rename(&from, &to).map_err(disk("rename", &from))?;
                }
                let brought = check_brought(&partial_path).map_err(|source| BringError::Check { machine, source })?;
                let text = notice(&facts, &brought);
                let (temp, notice_path) = (partial_path.join("NOTICE.md.tmp"), partial_path.join(NOTICE));
                std::fs::write(&temp, text).map_err(disk("write", &temp))?;
                std::fs::rename(&temp, &notice_path).map_err(disk("rename", &temp))?;
                std::fs::rename(&partial_path, &done_path).map_err(disk("rename", &partial_path))
            })
            .await,
        )?;
        crate::log::say!(
            "humatad2: session {} runs here now; what {} still held of it was brought into {}",
            bringing.session,
            bringing.machine,
            done.display()
        );
        Ok(done.to_owned())
    }
}

/// What the notice says, besides what was brought.
struct Facts {
    machine: String,
    operation: String,
    session: String,
    more: More,
    summary: Option<pb::ArchiveSummary>,
}

/// The notice that goes into a side folder, as markdown.
fn notice(facts: &Facts, brought: &Brought) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "# What {} still held of session {}\n", facts.machine, facts.session);
    let _ = writeln!(
        text,
        "Session {} was restored on this Mac (restore operation {}) while {} was away. When {} came back, \
         its copy held more than the restore had. Humata kept that here, checked it against what {} sent, \
         and then let {} release its copy.\n",
        facts.session, facts.operation, facts.machine, facts.machine, facts.machine, facts.machine
    );
    let _ = writeln!(text, "## What was brought\n");
    let _ = writeln!(text, "- `stage/records.sqlite`: the session's records");
    for rel in &brought.agent_files {
        let _ = writeln!(text, "- `stage/agent/{rel}`: the agent's conversation");
    }
    for rel in &brought.tabs {
        let _ = writeln!(text, "- `stage/tabs/{rel}`: a shell tab's saved output");
    }
    if brought.folder {
        let _ = writeln!(text, "- `folder/`: the session's folder as {} left it", facts.machine);
    } else {
        let _ = writeln!(text, "- no session folder: {} had already freed it", facts.machine);
    }
    let _ = writeln!(text, "\n{} files, {} bytes in all.\n", brought.files, brought.bytes);

    let _ = writeln!(text, "## What the comparison found\n");
    for extra in &facts.more.0 {
        let _ = writeln!(text, "- {extra}");
    }

    if let Some(summary) = &facts.summary {
        for kept in &summary.repos {
            let named = kept
                .repo
                .as_ref()
                .map(|repo| format!("{}/{}", repo.owner, repo.name))
                .unwrap_or_default();
            let pushed_past = facts
                .more
                .0
                .iter()
                .any(|extra| matches!(extra, Extra::PushedPast { repo } if *repo == named));
            let _ = writeln!(text, "\n## {named} (branch `{}`)\n", kept.branch);
            if pushed_past {
                let _ = writeln!(
                    text,
                    "{} pushed this branch past what the restore here cloned (up to `{}`): that work is on the remote, not lost.\n",
                    facts.machine, kept.last_pushed_sha
                );
            }
            if kept.files.is_empty() && kept.unpushed_subjects.is_empty() {
                let _ = writeln!(text, "No changed files and no unpushed commits.");
                continue;
            }
            let held = if brought.folder {
                "These are in `folder/`, as the session's folder was brought."
            } else {
                "These are gone: the session's folder was freed before it was brought, so only the paths and subjects below remain."
            };
            let _ = writeln!(text, "{held}\n");
            if !kept.files.is_empty() {
                let _ = writeln!(text, "Changed files:\n");
                for file in &kept.files {
                    let _ = writeln!(text, "- `{}` (+{} -{})", file.path, file.added, file.removed);
                }
                let _ = writeln!(text);
            }
            if !kept.unpushed_subjects.is_empty() {
                let _ = writeln!(text, "Unpushed commits:\n");
                for subject in &kept.unpushed_subjects {
                    let _ = writeln!(text, "- {subject}");
                }
            }
        }
    }

    let _ = writeln!(text, "\n## Discarding it\n");
    let _ = writeln!(text, "Delete this folder. Nothing in Humata refers to it.");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brought(folder: bool) -> Brought {
        Brought {
            files: 3,
            bytes: 30,
            folder,
            agent_files: vec!["conv.jsonl".to_owned()],
            tabs: vec!["tb_1.scrollback".to_owned()],
        }
    }

    fn facts(last_pushed: &str, extras: Vec<Extra>) -> Facts {
        Facts {
            machine: "box-1".to_owned(),
            operation: "op_7".to_owned(),
            session: "se_1".to_owned(),
            more: More(extras),
            summary: Some(pb::ArchiveSummary {
                repos: vec![pb::archive_summary::Repo {
                    repo: Some(pb::RepoRef { owner: "acme".to_owned(), name: "api".to_owned() }),
                    branch: "cache".to_owned(),
                    last_pushed_sha: last_pushed.to_owned(),
                    files: vec![pb::FileChange { path: "src/wip.txt".to_owned(), added: 2, removed: 1 }],
                    unpushed_subjects: vec!["warm the cache".to_owned()],
                }],
                ..pb::ArchiveSummary::default()
            }),
        }
    }

    #[test]
    fn the_notice_names_what_was_brought_by_its_path() {
        let text = notice(&facts("abc", vec![Extra::LaterTurns(2)]), &brought(false));
        for path in ["stage/records.sqlite", "stage/agent/conv.jsonl", "stage/tabs/tb_1.scrollback"] {
            assert!(text.contains(path), "{path} missing from {text}");
        }
        assert!(text.contains("op_7") && text.contains("box-1"));
    }

    #[test]
    fn work_without_a_brought_folder_is_named_and_called_gone() {
        let text = notice(&facts("abc", vec![Extra::Changed { repo: "acme/api".to_owned(), files: 1 }]), &brought(false));
        assert!(text.contains("src/wip.txt"));
        assert!(text.contains("warm the cache"));
        assert!(text.contains("gone"));
    }

    #[test]
    fn work_in_a_brought_folder_is_not_called_gone() {
        let text = notice(&facts("abc", vec![Extra::Changed { repo: "acme/api".to_owned(), files: 1 }]), &brought(true));
        assert!(text.contains("src/wip.txt"));
        assert!(text.contains("folder/"));
        assert!(!text.contains("gone"));
    }

    #[test]
    fn a_branch_pushed_past_the_restore_is_on_the_remote() {
        let past = vec![Extra::PushedPast { repo: "acme/api".to_owned() }];
        let text = notice(&facts("abc123", past), &brought(false));
        assert!(text.contains("abc123"));
        assert!(text.contains("remote"));
    }

    #[test]
    fn link_errors_are_told_from_the_rest() {
        let link = BringError::Call {
            machine: "box-1".to_owned(),
            doing: "stage",
            source: Status::unavailable("away"),
        };
        let other = BringError::Call {
            machine: "box-1".to_owned(),
            doing: "stage",
            source: Status::internal("broke"),
        };
        assert!(link.is_link());
        assert!(!other.is_link());
    }
}
