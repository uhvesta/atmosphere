//! File locations (CLI.md §5.5): `<machine>:<session>/<owner>/<repo>/<path>`,
//! `<machine>:shared/<path>`, `<machine>:channel/<path>` and
//! `<machine>:main/<owner>/<repo>/<path>`, and a session's own folder,
//! where a Desk session's documents sit: `<machine>:folder/<session>/<path>`
//! (D382), and the place files from another machine land,
//! `<machine>:received/<source machine>/<path>` (PL-4). Inside a session a bare path is
//! the agent's own clone, filled in from who the caller is, except `shared`,
//! `shared/` and `shared/<path>`, the channel's shared folder on the agent's
//! machine (D777). The clone's own folder of that name is `./shared`.

use humata2_proto::humata::v1 as pb;
use pb::file_location::Root;

use crate::call::Ctx;
use crate::console::Console;
use crate::daemon::{Daemon, rpc};
use crate::error::CliError;
use crate::principal::Principal;
use crate::resolve;

const EXPECTED: &str = "<machine>:<session>/<owner>/<repo>/<path>, <machine>:shared/<path>, \
                        <machine>:channel/<path>, <machine>:main/<owner>/<repo>/<path> or \
                        <machine>:folder/<session>/<path> or \
                        <machine>:received/<source machine>/<path>";

impl<D: Daemon, C: Console> Ctx<'_, D, C> {
    pub async fn location(&self, text: &str) -> Result<pb::FileLocation, CliError> {
        let bad = || CliError::BadValue {
            field: "location",
            value: text.into(),
            expected: EXPECTED,
        };
        let Some((machine, rest)) = text.split_once(':') else {
            return match self.principal {
                Principal::Agent { .. } if is_bare_shared(text) => self.own_shared(text).await,
                Principal::Agent { .. } => self.own_clone(text).await,
                _ => Err(bad()),
            };
        };
        let machine = Some(self.machine(machine).await?);
        let (head, tail) = rest.split_once('/').unwrap_or((rest, ""));
        let (root, path) = match head {
            "shared" => (
                Root::ChannelShared(pb::ChannelShared {
                    channel: Some(self.this_channel().await?),
                }),
                tail,
            ),
            "channel" => (
                Root::ChannelFolder(pb::ChannelFolder {
                    channel: Some(self.this_channel().await?),
                }),
                tail,
            ),
            "folder" => {
                let (session, path) = tail.split_once('/').unwrap_or((tail, ""));
                if session.is_empty() {
                    return Err(bad());
                }
                (
                    Root::SessionFolder(pb::SessionFolder {
                        session: Some(self.session(session).await?),
                    }),
                    path,
                )
            }
            "received" => {
                let (from, path) = tail.split_once('/').unwrap_or((tail, ""));
                if from.is_empty() {
                    return Err(bad());
                }
                (
                    Root::Received(pb::MachineReceived {
                        from: Some(self.machine(from).await?),
                    }),
                    path,
                )
            }
            "main" => {
                let (repo, path) = split_repo(tail).ok_or_else(bad)?;
                (
                    Root::MainClone(pb::MainClone {
                        channel: Some(self.this_channel().await?),
                        repo: Some(repo),
                    }),
                    path,
                )
            }
            session => {
                if session.is_empty() {
                    return Err(bad());
                }
                let (repo, path) = split_repo(tail).ok_or_else(bad)?;
                (
                    Root::SessionClone(pb::SessionClone {
                        session: Some(self.session(session).await?),
                        repo: Some(repo),
                    }),
                    path,
                )
            }
        };
        Ok(pb::FileLocation {
            machine,
            root: Some(root),
            path: path.into(),
        })
    }
}

impl<D: Daemon, C: Console> Ctx<'_, D, C> {
    /// A bare path inside a session: the agent's own clone when its session
    /// has one repository, its own folder when it has none (a Desk
    /// session, D382). The daemon serves only a location that names the
    /// session and the repo, so both are filled in here from who the
    /// caller is (D655). `./name` says "a file or folder of the clone, whatever
    /// its name" and `name/` does too, except `shared/` (D756, D777); the
    /// daemon takes the path without either.
    async fn own_clone(&self, path: &str) -> Result<pb::FileLocation, CliError> {
        let path = path.strip_prefix("./").unwrap_or(path).trim_end_matches('/');
        let me: pb::WhoAmIResponse = self.call(rpc::daemon::WHO_AM_I, &pb::WhoAmIRequest {}).await?;
        let session = match me.actor.and_then(|actor| actor.actor) {
            Some(pb::actor::Actor::Agent(agent)) => agent.session,
            _ => None,
        }
        .filter(|session| !session.value.is_empty())
        .ok_or(CliError::Missing {
            flag: "a location with its machine: write it as <machine>:<location>",
        })?;
        let record: pb::Session = self
            .call(
                rpc::session::GET_SESSION,
                &pb::GetSessionRequest {
                    id: Some(session.clone()),
                },
            )
            .await?;
        let mut repos: Vec<pb::RepoRef> = record.repos.into_iter().filter_map(|repo| repo.repo).collect();
        let root = match repos.len() {
            0 => Root::SessionFolder(pb::SessionFolder { session: Some(session) }),
            1 => Root::SessionClone(pb::SessionClone {
                session: Some(session),
                repo: repos.pop(),
            }),
            _ => {
                return Err(CliError::WhichClone {
                    path: path.into(),
                    repos: repos.iter().map(|repo| format!("{}/{}", repo.owner, repo.name)).collect(),
                });
            }
        };
        Ok(pb::FileLocation {
            machine: None,
            root: Some(root),
            path: path.into(),
        })
    }
}

/// Whether an agent's location with no `:` starts with the part `shared`
/// (`shared`, `shared/`, `shared/a/b`), the channel's shared folder on its
/// own machine (D777). `./shared` and `sharedstuff/` are not.
pub(crate) fn is_bare_shared(text: &str) -> bool {
    !text.contains(':') && text.split('/').next() == Some("shared")
}

impl<D: Daemon, C: Console> Ctx<'_, D, C> {
    /// An agent's bare `shared/<path>`: the channel's shared folder, with no
    /// machine named, as a bare clone path names none, so the daemon the
    /// agent talks to fills in its own machine (D777).
    async fn own_shared(&self, text: &str) -> Result<pb::FileLocation, CliError> {
        let path = text.strip_prefix("shared").unwrap_or(text).trim_matches('/');
        Ok(pb::FileLocation {
            machine: None,
            root: Some(Root::ChannelShared(pb::ChannelShared {
                channel: Some(self.this_channel().await?),
            })),
            path: path.into(),
        })
    }
}

/// `owner/repo/rest` as the repo and the rest.
fn split_repo(text: &str) -> Option<(pb::RepoRef, &str)> {
    let mut parts = text.splitn(3, '/');
    let owner = parts.next()?;
    let name = parts.next()?;
    let repo = resolve::repo(&format!("{owner}/{name}")).ok()?;
    Some((repo, parts.next().unwrap_or("")))
}
