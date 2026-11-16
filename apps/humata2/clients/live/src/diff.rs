//! A session's changes as the Diff panel draws them, read through the
//! daemon's DiffService.

use humata2_clients::DaemonError;
use humata2_clients::model::{
    DiffBase, DiffLine, FileDiff, LineKind, PushChoice, PushPlan, PushRepo, RepoDiff, SessionId,
};
use humata2_grpc::humata::v1 as pb;

use crate::connection::{Shared, fresh_key};
use crate::map;

const WHAT: &str = "The session's changes";


fn wire_base(base: &DiffBase) -> Option<pb::DiffBase> {
    use pb::diff_base::Base;
    let base = match base {
        DiffBase::OriginMain => Base::OriginMain(pb::OriginMain {}),
        DiffBase::UncommittedOnly => Base::UncommittedOnly(pb::UncommittedOnly {}),
        DiffBase::SinceLastReviewed => Base::SinceLastReviewed(pb::SinceLastReviewed {}),
        DiffBase::SinceFork => Base::SinceFork(pb::SinceFork {}),
        DiffBase::OtherSession(other) => Base::OtherSession(pb::SessionId {
            value: other.as_str().to_owned(),
        }),
    };
    Some(pb::DiffBase { base: Some(base) })
}

fn line(kind: LineKind, old: u32, new: u32, text: &str) -> DiffLine {
    let (old, new) = match kind {
        LineKind::Context => (Some(old), Some(new)),
        LineKind::Added => (None, Some(new)),
        LineKind::Removed => (Some(old), None),
        LineKind::Hunk => (None, None),
    };
    DiffLine {
        kind,
        old,
        new,
        text: text.to_owned(),
    }
}

fn readable_base(base: &DiffBase) -> String {
    match base {
        DiffBase::OriginMain => "origin/main".to_owned(),
        DiffBase::UncommittedOnly => "uncommitted changes".to_owned(),
        DiffBase::SinceLastReviewed => "since last reviewed".to_owned(),
        DiffBase::SinceFork => "since fork".to_owned(),
        DiffBase::OtherSession(other) => format!("session {}", other.as_str()),
    }
}

fn push_repo(repo: &pb::push_plan::Repo) -> PushRepo {
    PushRepo {
        repo: repo.repo.as_ref().map(map::repo_text).unwrap_or_default(),
        branch: repo.branch.clone(),
        dirty_files: repo.dirty_files,
        unpushed_commits: repo.unpushed_commits,
        added: repo.added,
        removed: repo.removed,
        open_pull_request: repo.existing.as_ref().map(|existing| existing.number),
    }
}

impl Shared {
    /// A file in the session's clone of `repo` (`owner/name`), whole, as
    /// the editor and document tabs show it. Refuses a file that isn't text.
    pub(crate) async fn read_session_file(
        &self,
        session: &SessionId,
        repo: &str,
        path: &str,
    ) -> Result<String, DaemonError> {
        const WHAT: &str = "Reading the file";
        let (owner, name) = repo.split_once('/').unwrap_or(("", repo));
        let location = pb::FileLocation {
            machine: None,
            root: Some(pb::file_location::Root::SessionClone(pb::SessionClone {
                session: Some(pb::SessionId {
                    value: session.as_str().to_owned(),
                }),
                repo: Some(pb::RepoRef {
                    owner: owner.to_owned(),
                    name: name.to_owned(),
                }),
            })),
            path: path.to_owned(),
        };
        let mut chunks = self
            .file_service()
            .read_file(pb::ReadFileRequest {
                location: Some(location),
                ..pb::ReadFileRequest::default()
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        let mut bytes = Vec::new();
        while let Some(chunk) = chunks
            .message()
            .await
            .map_err(|status| self.error(&status, WHAT))?
        {
            bytes.extend_from_slice(&chunk.data);
            if chunk.last {
                break;
            }
        }
        String::from_utf8(bytes).map_err(|_| DaemonError::Refused {
            what: WHAT.to_owned(),
            reason: format!("{repo}/{path} isn't UTF-8 text, so it can't be shown as text; open it with Open big or in another app"),
        })
    }

    /// What pushing each of the session's repositories would do, with the
    /// token that spends the plan.
    pub(crate) async fn plan_push(&self, session: &SessionId) -> Result<PushPlan, DaemonError> {
        let plan = self
            .sessions()
            .plan_push(pb::PlanPushRequest {
                id: Some(pb::SessionId {
                    value: session.as_str().to_owned(),
                }),
            })
            .await
            .map_err(|status| self.error(&status, "Reading what pushing would do"))?
            .into_inner();
        let token = plan
            .token
            .map(|token| token.value)
            .filter(|token| !token.is_empty())
            .ok_or_else(|| DaemonError::Refused {
                what: "Reading what pushing would do".to_owned(),
                reason: "the daemon gave no plan to push with. Read the changes again; if it keeps happening, update the daemon".to_owned(),
            })?;
        Ok(PushPlan {
            token,
            repos: plan.repos.iter().map(push_repo).collect(),
        })
    }

    /// Commits, pushes and opens pull requests for the chosen repositories
    /// with the plan's token. Returns the pull request the daemon named.
    pub(crate) async fn push(
        &self,
        session: &SessionId,
        token: &str,
        choices: Vec<PushChoice>,
    ) -> Result<Option<u32>, DaemonError> {
        let mut repos = Vec::with_capacity(choices.len());
        for choice in choices {
            let repo = map::parse_repo(&choice.repo).ok_or_else(|| DaemonError::Refused {
                what: format!("Pushing {}", choice.repo),
                reason: format!(
                    "{} isn't a repository name in owner/name form. Read the changes again and push from the group the repository is in",
                    choice.repo
                ),
            })?;
            repos.push(pb::PushChoice {
                repo: Some(repo),
                commit_message: choice.commit_message,
                open_pull_request: choice.open_pull_request,
                pr_title: choice.title,
                pr_body: choice.body,
            });
        }
        let names = repos
            .iter()
            .filter_map(|repo| repo.repo.as_ref().map(map::repo_text))
            .collect::<Vec<_>>()
            .join(", ");
        let what = format!("Push {names}");
        let operation = self
            .sessions()
            .push_and_open_pull_requests(pb::PushAndOpenPullRequestsRequest {
                idempotency_key: fresh_key(),
                id: Some(pb::SessionId {
                    value: session.as_str().to_owned(),
                }),
                repos,
                plan: Some(pb::PlanToken {
                    value: token.to_owned(),
                }),
                from_view: None,
            })
            .await
            .map_err(|status| self.error(&status, &what))?
            .into_inner();
        let done = self.finish(operation, &what).await?;
        Ok(match done.result {
            Some(pb::operation::Result::PullRequest(pull_request)) => Some(pull_request.number),
            _ => None,
        })
    }

    /// Remembers what the session's changes are now as reviewed.
    pub(crate) async fn mark_reviewed(&self, session: &SessionId) -> Result<(), DaemonError> {
        self.diffs()
            .mark_reviewed(pb::MarkReviewedRequest {
                idempotency_key: fresh_key(),
                session: Some(pb::SessionId {
                    value: session.as_str().to_owned(),
                }),
                // The app is the person: no presence proof is sent.
                presence: None,
                shown_tree: Vec::new(),
            })
            .await
            .map_err(|status| self.error(&status, "Marking the changes reviewed"))?;
        Ok(())
    }

    /// Every repository's changes in the session's clones, against where
    /// its branch started. Each file is fetched whole, page after page.
    pub(crate) async fn session_diffs(&self, session: &SessionId, base: &DiffBase) -> Result<Vec<RepoDiff>, DaemonError> {
        let id = Some(pb::SessionId {
            value: session.as_str().to_owned(),
        });
        let listing = self
            .diffs()
            .get_diff(pb::GetDiffRequest {
                session: id.clone(),
                base: wire_base(base),
                repos: Vec::new(),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        let mut diffs = Vec::new();
        for listed in &listing.repos {
            let repo = listed.repo.clone().unwrap_or_default();
            let mut files = Vec::new();
            for summary in &listed.files {
                let mut lines = Vec::new();
                if !summary.binary {
                    let mut from = None;
                    loop {
                        let page = self
                            .diffs()
                            .get_file_diff(pb::GetFileDiffRequest {
                                session: id.clone(),
                                base: wire_base(base),
                                file: Some(pb::RepoPath {
                                    repo: Some(repo.clone()),
                                    path: summary.path.clone(),
                                }),
                                trees: listed.trees.clone(),
                                from,
                                ..pb::GetFileDiffRequest::default()
                            })
                            .await
                            .map_err(|status| self.error(&status, WHAT))?
                            .into_inner();
                        for hunk in &page.hunks {
                            if !hunk.continued {
                                lines.push(line(LineKind::Hunk, 0, 0, &hunk.header));
                            }
                            for each in &hunk.lines {
                                let kind = match pb::diff_line::Kind::try_from(each.kind) {
                                    Ok(pb::diff_line::Kind::Added) => LineKind::Added,
                                    Ok(pb::diff_line::Kind::Removed) => LineKind::Removed,
                                    _ => LineKind::Context,
                                };
                                lines.push(line(kind, each.old_line, each.new_line, &each.text));
                            }
                        }
                        match page.next {
                            Some(next) => from = Some(next),
                            None => break,
                        }
                    }
                }
                files.push(FileDiff {
                    path: summary.path.clone(),
                    added: summary.added,
                    removed: summary.removed,
                    lines,
                });
            }
            diffs.push(RepoDiff {
                session: session.clone(),
                repo: map::repo_text(&repo),
                base: if listed.label.is_empty() {
                    readable_base(base)
                } else {
                    listed.label.clone()
                },
                files,
                comments: Vec::new(),
            });
        }
        Ok(diffs)
    }
}
