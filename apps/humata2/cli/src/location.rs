//! File locations (CLI.md §5.5): `<machine>:<session>/<owner>/<repo>/<path>`,
//! `<machine>:shared/<path>`, `<machine>:channel/<path>` and
//! `<machine>:main/<owner>/<repo>/<path>`. Inside a session a bare path is
//! the agent's own clone, which the daemon knows from the caller.

use humata2_proto::humata::v1 as pb;
use pb::file_location::Root;

use crate::call::Ctx;
use crate::console::Console;
use crate::daemon::Daemon;
use crate::error::CliError;
use crate::principal::Principal;
use crate::resolve;

const EXPECTED: &str = "<machine>:<session>/<owner>/<repo>/<path>, <machine>:shared/<path>, \
                        <machine>:channel/<path> or <machine>:main/<owner>/<repo>/<path>";

impl<D: Daemon, C: Console> Ctx<'_, D, C> {
    pub async fn location(&self, text: &str) -> Result<pb::FileLocation, CliError> {
        let bad = || CliError::BadValue {
            field: "location",
            value: text.into(),
            expected: EXPECTED,
        };
        let Some((machine, rest)) = text.split_once(':') else {
            return match self.principal {
                Principal::Agent { .. } => Ok(pb::FileLocation {
                    machine: None,
                    root: Some(Root::SessionClone(pb::SessionClone::default())),
                    path: text.into(),
                }),
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

/// `owner/repo/rest` as the repo and the rest.
fn split_repo(text: &str) -> Option<(pb::RepoRef, &str)> {
    let mut parts = text.splitn(3, '/');
    let owner = parts.next()?;
    let name = parts.next()?;
    let repo = resolve::repo(&format!("{owner}/{name}")).ok()?;
    Some((repo, parts.next().unwrap_or("")))
}
