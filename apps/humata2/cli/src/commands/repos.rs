//! `humata repos search|list|remove|forget-all|scope` (RL-4, RL-5, RL-7,
//! RL-11): the repo library and
//! the one repo search. A search with no flags uses the saved scope; the
//! flags widen or change it for that one call and are never saved; only
//! `repos scope set` saves it (D567). `repos forget-all` asks first.

use humata2_proto::humata::v1 as pb;
use pb::repo_scope::Kind;

use super::{pages, reply};
use crate::args::{RepoScopeCommand, ReposCommand};
use crate::call::{Ctx, Done, Outcome};
use crate::console::Console;
use crate::daemon::{Daemon, rpc};
use crate::error::CliError;
use crate::resolve;

pub(super) async fn repos<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: ReposCommand,
) -> Result<Done, CliError> {
    match command {
        ReposCommand::Search {
            query,
            owners,
            all_of_github,
            your_repos,
        } => search(ctx, &query, owners, all_of_github, your_repos).await,
        ReposCommand::List => {
            reply(
                ctx,
                rpc::repos::LIST_REPO_LIBRARY,
                &pb::ListRepoLibraryRequest {},
            )
            .await
        }
        ReposCommand::Remove { repo } => {
            let request = pb::RemoveFromRepoLibraryRequest {
                repo: Some(resolve::repo(&repo)?),
            };
            ctx.change(rpc::repos::REMOVE_FROM_REPO_LIBRARY, &request).await
        }
        ReposCommand::ForgetAll => forget_all(ctx).await,
        ReposCommand::Scope(RepoScopeCommand::Show) => {
            reply(ctx, rpc::repos::GET_REPO_SCOPE, &pb::GetRepoScopeRequest {}).await
        }
        ReposCommand::Scope(RepoScopeCommand::Set {
            owners,
            no_include_me,
        }) => {
            let request = pb::UpdateRepoScopeRequest {
                scope: Some(saved_scope(owners, !no_include_me)?),
            };
            ctx.change(rpc::repos::UPDATE_REPO_SCOPE, &request).await
        }
    }
}

async fn search<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    words: &[String],
    owners: Vec<String>,
    all_of_github: bool,
    your_repos: bool,
) -> Result<Done, CliError> {
    let query = words
        .iter()
        .map(|word| word.trim())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if query.is_empty() {
        return Err(CliError::BadValue {
            field: "search words",
            value: words.join(" "),
            expected: "at least one word to look for",
        });
    }
    let scope = scope(owners, all_of_github, your_repos)?;
    pages(ctx, rpc::repos::SEARCH_REPOS, "github", |page| {
        pb::SearchReposRequest {
            query: query.clone(),
            scope: scope.clone(),
            page,
        }
    })
    .await
    .map(Done::reply)
}

/// The scope for this search only; none keeps the saved one.
fn scope(
    owners: Vec<String>,
    all_of_github: bool,
    your_repos: bool,
) -> Result<Option<pb::RepoScope>, CliError> {
    let kind = if all_of_github {
        Kind::AllOfGithub(true)
    } else if !owners.is_empty() {
        Kind::Owners(pb::RepoOwners {
            owners: owner_names(owners)?,
            // D474: listed owners plus your own repos, as the saved scope defaults.
            include_me: true,
        })
    } else if your_repos {
        Kind::YourRepos(true)
    } else {
        return Ok(None);
    };
    Ok(Some(pb::RepoScope { kind: Some(kind) }))
}

/// The scope `repos scope set` saves: the owners, plus your own repos unless
/// left out; with no owner, just your own repos.
fn saved_scope(owners: Vec<String>, include_me: bool) -> Result<pb::RepoScope, CliError> {
    let owners = owner_names(owners)?;
    let kind = if !owners.is_empty() {
        Kind::Owners(pb::RepoOwners { owners, include_me })
    } else if include_me {
        Kind::YourRepos(true)
    } else {
        return Err(CliError::BadValue {
            field: "--no-include-me",
            value: "with no --owner".into(),
            expected: "at least one --owner to search when your own repos are left out",
        });
    };
    Ok(pb::RepoScope { kind: Some(kind) })
}

fn owner_names(owners: Vec<String>) -> Result<Vec<String>, CliError> {
    let mut named = Vec::new();
    for owner in owners {
        let owner = owner.trim().to_owned();
        if owner.is_empty() || owner.contains('/') {
            return Err(CliError::BadValue {
                field: "--owner",
                value: owner,
                expected: "a GitHub user or organization such as acme",
            });
        }
        named.push(owner);
    }
    Ok(named)
}

/// Forgets the whole library (RL-4) once the user said yes to how many repos
/// go; channels, sessions, clones and GitHub keep everything. --dry-run only
/// says it.
async fn forget_all<D: Daemon, C: Console>(ctx: &Ctx<'_, D, C>) -> Result<Done, CliError> {
    let list = rpc::repos::LIST_REPO_LIBRARY;
    let bytes = ctx.raw(list, &pb::ListRepoLibraryRequest {}).await?;
    let library: pb::ListRepoLibraryResponse = crate::call::decode(list, &bytes)?;
    let plan = serde_json::json!({
        "forget": library.repos.len(),
        "kept": "every channel, session and clone; nothing is deleted on disk or on GitHub",
    });
    if ctx.flags.dry_run {
        return Ok(Done {
            result: plan,
            outcome: Outcome::Planned,
        });
    }
    ctx.confirm(&plan)?;
    reply(
        ctx,
        rpc::repos::FORGET_REPO_LIBRARY,
        &pb::ForgetRepoLibraryRequest {},
    )
    .await
}
