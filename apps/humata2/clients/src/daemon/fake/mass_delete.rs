//! MASSDELETE on the fake daemon: one plan for many sessions, one token, one
//! delete. A delete whose sessions changed since the plan, or that the test
//! marked with `change_after_plan`, deletes nothing and answers the new plan
//! (NV-27).

use super::{FakeDaemon, Inner};
use crate::daemon::{DaemonError, Event, Reply, UnpushedWork};
use crate::desk;
use crate::mass_delete::{
    DeleteScope, DeleteSessionsOutcome, DeleteSessionsPlan, EndingState, LeftAlone, SessionToDelete,
    SessionWork, TerminalEnding,
};
use crate::model::{ForkKind, Session, SessionId, Status};

/// What the fake keeps of the plan it last gave out.
#[derive(Default)]
pub(super) struct State {
    /// The last plan: its token, its scope and the sessions it listed.
    held: Option<Held>,
    /// How many plans were given out, for their tokens.
    issued: u32,
    /// Set by `change_after_plan`: the next delete answers Changed.
    change_next: bool,
}

struct Held {
    token: String,
    scope: DeleteScope,
    sessions: Vec<SessionToDelete>,
}

/// What a plan lists, before it is given a token.
struct Computed {
    sessions: Vec<SessionToDelete>,
    left_alone: Vec<LeftAlone>,
    forks_staying: Vec<SessionWork>,
    desk_sessions_left_out: u32,
}

fn not_found(what: String) -> DaemonError {
    DaemonError::NotFound { what }
}

fn refused(what: &str, reason: String) -> DaemonError {
    DaemonError::Refused {
        what: what.to_owned(),
        reason,
    }
}

/// The sessions the scope names, live and archived, Desk ones included.
fn candidates<'a>(inner: &'a Inner, scope: &DeleteScope) -> Result<Vec<&'a Session>, DaemonError> {
    match scope {
        DeleteScope::Picked(ids) => ids
            .iter()
            .map(|id| {
                inner
                    .workspace
                    .session(id)
                    .ok_or_else(|| not_found(format!("Session {id}")))
            })
            .collect(),
        DeleteScope::Channel(channel) => Ok(inner
            .workspace
            .sessions
            .iter()
            .filter(|session| &session.channel == channel)
            .collect()),
        DeleteScope::Machine(machine) => {
            if inner.workspace.machine(machine).is_none() {
                return Err(not_found(format!("Host {machine}")));
            }
            Ok(inner
                .workspace
                .sessions
                .iter()
                .filter(|session| &session.machine == machine)
                .collect())
        }
        DeleteScope::All => Ok(inner.workspace.sessions.iter().collect()),
    }
}

/// Whether the session's host is known and doesn't answer.
fn machine_offline(inner: &Inner, session: &Session) -> bool {
    inner
        .workspace
        .machine(&session.machine)
        .is_some_and(|machine| !machine.online)
}

fn unpushed_of(inner: &Inner, session: &SessionId) -> Vec<UnpushedWork> {
    inner
        .unpushed
        .get(session)
        .into_iter()
        .flatten()
        .map(|(repo, what)| UnpushedWork {
            repo: repo.clone(),
            what: what.clone(),
        })
        .collect()
}

/// The terminals a delete ends, and whether the session's agent has exited.
/// The agent is named by its command; the shells by their names.
fn endings(session: &Session) -> (Vec<TerminalEnding>, bool) {
    if session.archived {
        return (Vec::new(), false);
    }
    if matches!(session.status, Status::Exited { .. }) {
        return (Vec::new(), true);
    }
    let agent_state = match session.status {
        Status::NeedsYou => EndingState::WaitingForYou,
        Status::Working => EndingState::Working,
        _ => EndingState::Idle,
    };
    let mut terminals = vec![TerminalEnding {
        name: session.agent.command().to_owned(),
        state: agent_state,
    }];
    terminals.extend(session.shells.iter().map(|name| TerminalEnding {
        name: name.clone(),
        state: EndingState::Idle,
    }));
    (terminals, false)
}

fn to_delete(inner: &Inner, session: &Session) -> SessionToDelete {
    let (terminals, exited) = endings(session);
    SessionToDelete {
        session: session.id.clone(),
        title: session.title.clone(),
        channel: session.channel.clone(),
        machine: session.machine.clone(),
        archived: session.archived,
        unpushed: unpushed_of(inner, &session.id),
        running_terminals: u32::try_from(terminals.len()).unwrap_or(u32::MAX),
        goes_with: None,
        terminals,
        exited,
    }
}

/// The fake keeps no clone state, so a session's counts stay at zero.
fn session_work(session: &Session) -> SessionWork {
    SessionWork {
        session: session.id.clone(),
        title: session.title.clone(),
        dirty_files: 0,
        unpushed_commits: 0,
    }
}

/// What a scope would delete, leave alone and keep running. Desk sessions
/// are counted and left out; sessions on an unreachable host are left alone.
fn compute(inner: &Inner, scope: &DeleteScope) -> Result<Computed, DaemonError> {
    let (desk_sessions, channel_sessions): (Vec<&Session>, Vec<&Session>) = candidates(inner, scope)?
        .into_iter()
        .partition(|session| desk::is_desk(session));

    let mut sessions = Vec::new();
    let mut left_alone: Vec<LeftAlone> = Vec::new();
    for session in channel_sessions {
        if machine_offline(inner, session) {
            let work = session_work(session);
            match left_alone
                .iter_mut()
                .find(|left| left.machine == session.machine)
            {
                Some(left) => left.sessions.push(work),
                None => left_alone.push(LeftAlone {
                    machine: session.machine.clone(),
                    sessions: vec![work],
                }),
            }
        } else {
            sessions.push(to_delete(inner, session));
        }
    }

    // Recursively pull in non-archived, non-Desk context-only forks whose
    // parent is in the delete set (D187).
    loop {
        let to_add: Vec<(&Session, SessionId)> = inner
            .workspace
            .sessions
            .iter()
            .filter_map(|session| {
                if session.archived
                    || desk::is_desk(session)
                    || session.fork != Some(ForkKind::Conversation)
                    || machine_offline(inner, session)
                    || sessions.iter().any(|planned| planned.session == session.id)
                {
                    return None;
                }
                let parent = session.parent.as_ref()?;
                if sessions.iter().any(|planned| &planned.session == parent) {
                    Some((session, parent.clone()))
                } else {
                    None
                }
            })
            .collect();
        if to_add.is_empty() {
            break;
        }
        for (session, parent) in to_add {
            let mut planned = to_delete(inner, session);
            planned.goes_with = Some(parent);
            sessions.push(planned);
        }
    }

    // A non-conversation fork of a deleted session that isn't deleted keeps running.
    let deleted: Vec<&SessionId> = sessions.iter().map(|session| &session.session).collect();
    let forks_staying = inner
        .workspace
        .sessions
        .iter()
        .filter(|session| {
            !session.archived
                && session.fork != Some(ForkKind::Conversation)
                && !deleted.contains(&&session.id)
                && session
                    .parent
                    .as_ref()
                    .is_some_and(|parent| deleted.contains(&parent))
        })
        .map(session_work)
        .collect();

    Ok(Computed {
        sessions,
        left_alone,
        forks_staying,
        desk_sessions_left_out: u32::try_from(desk_sessions.len()).unwrap_or(u32::MAX),
    })
}

/// Gives the computed plan a new token and holds it for the next delete.
fn issue(inner: &mut Inner, scope: DeleteScope, computed: Computed) -> DeleteSessionsPlan {
    inner.mass_delete.issued += 1;
    let token = format!("mass-delete-{}", inner.mass_delete.issued);
    let needs_count = matches!(scope, DeleteScope::All);
    inner.mass_delete.held = Some(Held {
        token: token.clone(),
        scope,
        sessions: computed.sessions.clone(),
    });
    DeleteSessionsPlan {
        sessions: computed.sessions,
        left_alone: computed.left_alone,
        forks_staying: computed.forks_staying,
        desk_sessions_left_out: computed.desk_sessions_left_out,
        token,
        needs_count,
    }
}

fn parse_commits(what: &str) -> Option<u32> {
    let words: Vec<&str> = what.split_whitespace().collect();
    for pair in words.windows(2) {
        let word = pair[1].trim_matches(|c: char| !c.is_ascii_alphabetic());
        if word.eq_ignore_ascii_case("commit") || word.eq_ignore_ascii_case("commits") {
            if let Ok(count) = pair[0].parse::<u32>() {
                return Some(count);
            }
        }
    }
    None
}

fn parse_branch(what: &str) -> Option<&str> {
    let (_, after) = what.split_once(" on ")?;
    let branch = after
        .split_whitespace()
        .next()?
        .trim_end_matches(|c: char| matches!(c, ',' | '.' | ';' | ':'));
    (!branch.is_empty()).then_some(branch)
}

fn unpushed_change_line(inner: &Inner, prev: &SessionToDelete, curr: &SessionToDelete) -> String {
    for item in &curr.unpushed {
        let Some(now_commits) = parse_commits(&item.what) else {
            continue;
        };
        let then_commits = prev
            .unpushed
            .iter()
            .find(|old| old.repo == item.repo)
            .and_then(|old| parse_commits(&old.what))
            .unwrap_or(0);
        if let Some(added) = now_commits.checked_sub(then_commits).filter(|&n| n > 0) {
            let noun = if added == 1 { "commit" } else { "commits" };
            let branch = parse_branch(&item.what).or_else(|| {
                inner
                    .workspace
                    .session(&curr.session)
                    .map(|session| session.branch.as_str())
                    .filter(|branch| !branch.is_empty())
            });
            return match branch {
                Some(branch) => format!("{}: {added} new {noun} on {branch}.", curr.title),
                None => format!("{}: {added} new {noun}.", curr.title),
            };
        }
    }
    format!("{}: changed.", curr.title)
}

/// What differs between the sessions listed then and the sessions now.
fn changes_since(
    inner: &Inner,
    listed: &[SessionToDelete],
    now: &[SessionToDelete],
) -> Vec<String> {
    let mut changes = Vec::new();
    for curr in now {
        let Some(prev) = listed.iter().find(|s| s.session == curr.session) else {
            changes.push(format!("\"{}\" is now in this delete", curr.title));
            continue;
        };
        if prev.unpushed != curr.unpushed {
            changes.push(unpushed_change_line(inner, prev, curr));
        } else if prev != curr {
            changes.push(format!("{}: changed.", curr.title));
        }
    }
    for prev in listed {
        if !now.iter().any(|s| s.session == prev.session) {
            changes.push(format!("\"{}\" is no longer in it", prev.title));
        }
    }
    if changes.is_empty() {
        if let Some(first) = now.first() {
            changes.push(format!("{}: changed.", first.title));
        } else {
            changes.push("Plan changed.".to_owned());
        }
    }
    changes
}

pub(super) fn plan(daemon: &FakeDaemon, scope: DeleteScope) -> Reply<DeleteSessionsPlan> {
    daemon.reply(
        None,
        format!("plan delete sessions {scope:?}"),
        move |inner| {
            let computed = compute(inner, &scope)?;
            Ok(issue(inner, scope, computed))
        },
    )
}

pub(super) fn delete(
    daemon: &FakeDaemon,
    token: &str,
    delete_unpushed_work: bool,
    typed_count: u32,
) -> Reply<DeleteSessionsOutcome> {
    let token = token.to_owned();
    daemon.reply(
        None,
        format!("delete sessions {token}"),
        move |inner| {
            let Some((held_token, scope, listed)) = inner
                .mass_delete
                .held
                .as_ref()
                .map(|held| (held.token.clone(), held.scope.clone(), held.sessions.clone()))
            else {
                return Err(refused(
                    "Deleting the sessions",
                    "there is no plan to delete from. Plan the delete again and confirm it".to_owned(),
                ));
            };
            let forced = std::mem::take(&mut inner.mass_delete.change_next);
            let computed = compute(inner, &scope)?;

            if forced || held_token != token || computed.sessions != listed {
                let changes = changes_since(inner, &listed, &computed.sessions);
                let plan = issue(inner, scope, computed);
                return Ok(DeleteSessionsOutcome::Changed { plan, changes });
            }

            let now: Vec<SessionId> = computed
                .sessions
                .iter()
                .map(|session| session.session.clone())
                .collect();
            let total = u32::try_from(computed.sessions.len()).unwrap_or(u32::MAX);
            if matches!(scope, DeleteScope::All) && typed_count != total {
                return Err(refused(
                    "Deleting every session",
                    format!("type {total} to confirm deleting all {total} sessions. Nothing was deleted"),
                ));
            }
            if !delete_unpushed_work
                && let Some(session) = computed
                    .sessions
                    .iter()
                    .find(|session| !session.unpushed.is_empty())
            {
                return Err(refused(
                    "Deleting the sessions",
                    format!(
                        "\"{}\" has work that exists only here. Push it first, or confirm deleting that work too and delete again",
                        session.title
                    ),
                ));
            }

            inner
                .workspace
                .sessions
                .retain(|session| !now.contains(&session.id));
            inner
                .workspace
                .requests
                .retain(|request| !now.contains(&request.session));
            inner.mass_delete.held = None;
            inner.emit(Event::Resync);
            Ok(DeleteSessionsOutcome::Deleted { sessions: total })
        },
    )
}

/// The next delete answers Changed, as if the sessions changed after the
/// plan, and deletes nothing.
pub(super) fn change_after_plan(daemon: &FakeDaemon) {
    daemon.inner().mass_delete.change_next = true;
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::Arc;

    use tca::TestClock;
    use tokio::runtime::Builder;

    use crate::desk;
    use crate::mass_delete::{DeleteScope, DeleteSessionsOutcome, EndingState};
    use crate::model::{ChannelId, MachineId, SessionId};
    use crate::{Daemon, FakeDaemon, Workspace, fixture};

    fn block_on<T>(future: impl Future<Output = T>) -> T {
        match Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime.block_on(future),
            Err(error) => panic!("a runtime starts: {error}"),
        }
    }

    /// The standard fixture with the Desk's sessions added.
    fn workspace() -> Workspace {
        let mut workspace = fixture::standard();
        fixture::desk(&mut workspace);
        workspace
    }

    fn daemon() -> FakeDaemon {
        FakeDaemon::new(workspace(), Arc::new(TestClock::new()))
    }

    fn ids(sessions: &[SessionId]) -> Vec<String> {
        sessions.iter().map(|id| id.as_str().to_owned()).collect()
    }

    #[test]
    fn planning_a_channel_leaves_out_desk_sessions_and_lists_an_offline_machine_as_left_alone() {
        let daemon = daemon();
        daemon.take_offline(&MachineId::new("gpu-box"));
        let scope = DeleteScope::Channel(ChannelId::new("web"));

        let plan = block_on(daemon.plan_delete_sessions(&scope)).expect("the plan is made");

        let planned: Vec<SessionId> = plan.sessions.iter().map(|s| s.session.clone()).collect();
        assert_eq!(ids(&planned), ["s-login", "s-passkey", "s-copy"]);
        assert!(plan.sessions.iter().all(|s| s.channel == ChannelId::new("web")));
        let left: Vec<SessionId> = plan.left_alone.iter().flat_map(|l| l.sessions.iter().map(|s| s.session.clone())).collect();
        assert_eq!(plan.left_alone.len(), 1);
        assert_eq!(plan.left_alone[0].machine, MachineId::new("gpu-box"));
        assert_eq!(ids(&left), ["s-email", "s-theme"]);
    }

    #[test]
    fn planning_everything_counts_the_desk_sessions_it_leaves_out() {
        let daemon = daemon();
        let desk_sessions = workspace()
            .sessions
            .iter()
            .filter(|session| desk::is_desk(session))
            .count();

        let plan = block_on(daemon.plan_delete_sessions(&DeleteScope::All)).expect("the plan is made");

        assert_eq!(plan.desk_sessions_left_out as usize, desk_sessions);
        assert!(plan.needs_count);
    }

    #[test]
    fn change_after_plan_answers_changed_and_deletes_nothing() {
        let daemon = daemon();
        let scope = DeleteScope::Channel(ChannelId::new("scratch"));
        let plan = block_on(daemon.plan_delete_sessions(&scope)).expect("the plan is made");
        let before = daemon.workspace().sessions.len();
        daemon.change_after_plan();

        let outcome = block_on(daemon.delete_sessions(&plan.token, false, 0)).expect("the delete answers");

        assert!(matches!(outcome, DeleteSessionsOutcome::Changed { .. }));
        assert_eq!(daemon.workspace().sessions.len(), before);
    }

    #[test]
    fn deleting_a_plan_removes_its_sessions() {
        let daemon = daemon();
        let scope = DeleteScope::Channel(ChannelId::new("scratch"));
        let plan = block_on(daemon.plan_delete_sessions(&scope)).expect("the plan is made");

        let outcome = block_on(daemon.delete_sessions(&plan.token, false, 0)).expect("the delete answers");

        assert_eq!(outcome, DeleteSessionsOutcome::Deleted { sessions: 2 });
        let remaining = daemon.workspace().sessions;
        assert!(!remaining.iter().any(|s| s.id == SessionId::new("s-notes")));
        assert!(!remaining.iter().any(|s| s.id == SessionId::new("s-old")));
    }

    #[test]
    fn a_terminal_waiting_for_you_is_listed_with_that_state() {
        let daemon = daemon();
        let plan = block_on(daemon.plan_delete_sessions(&DeleteScope::Channel(ChannelId::new("web"))))
            .expect("the plan is made");

        let passkey = plan
            .sessions
            .iter()
            .find(|s| s.session == SessionId::new("s-passkey"))
            .expect("s-passkey is planned");
        assert_eq!(passkey.terminals.first().map(|t| t.state), Some(EndingState::WaitingForYou));
        assert!(!passkey.exited);
    }

    #[test]
    fn conversation_forks_go_with_their_parent_while_code_forks_stay() {
        let daemon = daemon();
        let plan = block_on(daemon.plan_delete_sessions(&DeleteScope::Picked(vec![
            SessionId::new("s-passkey"),
        ])))
        .expect("the plan is made");

        let planned: Vec<SessionId> = plan.sessions.iter().map(|s| s.session.clone()).collect();
        assert_eq!(ids(&planned), ["s-passkey", "s-copy"]);
        assert_eq!(plan.sessions[0].goes_with, None);
        assert_eq!(plan.sessions[1].goes_with, Some(SessionId::new("s-passkey")));
        assert!(plan.forks_staying.is_empty());

        let login_plan = block_on(daemon.plan_delete_sessions(&DeleteScope::Picked(vec![
            SessionId::new("s-login"),
        ])))
        .expect("the login plan is made");
        let staying: Vec<SessionId> = login_plan
            .forks_staying
            .iter()
            .map(|s| s.session.clone())
            .collect();
        assert_eq!(ids(&staying), ["s-passkey", "s-email"]);
    }

    #[test]
    fn deleting_a_plan_withdraws_its_sessions_requests() {
        let daemon = daemon();
        assert!(
            daemon
                .workspace()
                .requests
                .iter()
                .any(|r| r.session == SessionId::new("s-passkey"))
        );
        let plan = block_on(daemon.plan_delete_sessions(&DeleteScope::Picked(vec![
            SessionId::new("s-passkey"),
        ])))
        .expect("the plan is made");

        let outcome = block_on(daemon.delete_sessions(&plan.token, false, 0))
            .expect("the delete answers");
        assert_eq!(outcome, DeleteSessionsOutcome::Deleted { sessions: 2 });
        assert!(
            !daemon
                .workspace()
                .requests
                .iter()
                .any(|r| r.session == SessionId::new("s-passkey"))
        );
    }

    #[test]
    fn change_after_plan_reports_per_session_unpushed_commit_changes() {
        let daemon = daemon();
        daemon.set_unpushed(
            &SessionId::new("s-notes"),
            vec![(
                "scratch".to_owned(),
                "2 commits on uhvesta/notes never pushed".to_owned(),
            )],
        );
        let plan = block_on(daemon.plan_delete_sessions(&DeleteScope::Channel(
            ChannelId::new("scratch"),
        )))
        .expect("the plan is made");
        daemon.set_unpushed(
            &SessionId::new("s-notes"),
            vec![(
                "scratch".to_owned(),
                "3 commits on uhvesta/notes never pushed".to_owned(),
            )],
        );
        daemon.change_after_plan();

        let outcome = block_on(daemon.delete_sessions(&plan.token, true, 0))
            .expect("the delete answers");
        match outcome {
            DeleteSessionsOutcome::Changed { changes, .. } => {
                assert_eq!(changes, vec!["Shell: 1 new commit on uhvesta/notes."]);
            }
            DeleteSessionsOutcome::Deleted { .. } => panic!("expected Changed"),
        }
    }
}
