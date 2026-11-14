//! Plan mode: a planning session with the code reads a copy that follows
//! the repository and changes none of it, and still proposes; a working
//! session is held to reading while its plan mode is on.

use std::path::{Path, PathBuf};

use humata_wire::{Client, pb};

use super::{
    after_turns, board, commit, daemon, git_repository, project, proposals_of, send, start_in,
    started, update,
};
use crate::hosts::Hosts;

/// Tries to change the code every way the scripted agent can, and proposes
/// a task.
const TRIES: &str = r#"{"rules": [{"when": "", "steps": [
    {"step": "write_file", "path": "README.md", "contents": "changed"},
    {"step": "write_file", "path": "new.txt", "contents": "new"},
    {"step": "shell", "command": "rm README.md", "output": "removed"},
    {"step": "tool", "tool": "propose_task",
     "arguments": {"title": "Log in", "spec": "With email."}},
    {"step": "say", "text": "Proposed."}
]}]}"#;

fn planning(project: &str, with_code: bool) -> pb::request::Kind {
    let mut request = start_in(project);
    if let pb::request::Kind::StartSession(start) = &mut request {
        start.planning = true;
        start.with_code = with_code;
        // It would be allowed everything.
        start.approval_mode = pb::ApprovalMode::AllowEverything.into();
        start.backend_config = TRIES.to_owned();
    }
    request
}

fn head(repository: &Path) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

async fn listed(client: &Client, session: &str) -> pb::SessionInfo {
    let sessions = client
        .call(pb::request::Kind::ListSessions(pb::ListSessions {
            project_id: String::new(),
        }))
        .await;
    let Ok(pb::response::Kind::Sessions(sessions)) = sessions else {
        panic!("{sessions:?}")
    };
    sessions
        .sessions
        .into_iter()
        .find(|listed| listed.id == session)
        .unwrap_or_else(|| panic!("{session} isn't listed"))
}

async fn offers_plan(client: &Client, session: &str) -> bool {
    let commands = client
        .call(pb::request::Kind::GetChatCommands(pb::GetChatCommands {
            session_id: session.to_owned(),
        }))
        .await;
    match commands {
        Ok(pb::response::Kind::ChatCommands(commands)) => commands.plan,
        other => panic!("{other:?}"),
    }
}

fn notices(events: &[pb::SessionEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event.kind, Some(pb::session_event::Kind::Notice(_))))
        .count()
}

fn access(session: &str, level: pb::AccessLevel) -> pb::UpdateSession {
    pb::UpdateSession {
        session_id: session.to_owned(),
        access: Some(level.into()),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_planning_session_with_the_code_reads_the_latest_changes_nothing_and_still_proposes() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let client = daemon(state.path(), Hosts::default()).await;
    let project = project(&client, repository.path()).await;

    let session = started(&client, planning(&project.id, true)).await;
    assert_eq!(session.access(), pb::AccessLevel::Plan);
    assert!(!session.read_only);
    send(&client, &session.id, "plan").await;
    after_turns(&client, &session.id, 1).await;

    // Its copy is as the repository is, whatever it tried.
    let copy = PathBuf::from(&listed(&client, &session.id).await.working_directory);
    assert_ne!(copy, repository.path());
    assert_eq!(
        std::fs::read_to_string(copy.join("README.md"))
            .ok()
            .as_deref(),
        Some("hello\n")
    );
    assert!(!copy.join("new.txt").exists());

    // What it proposed is there to apply, and applying it makes the task.
    let [proposal] = proposals_of(&client, &session.id)
        .await
        .try_into()
        .unwrap_or_else(|proposals: Vec<pb::Proposal>| panic!("one proposal: {proposals:?}"));
    let applied = client
        .call(pb::request::Kind::ApplyProposal(pb::ApplyProposal {
            session_id: session.id.clone(),
            proposal_id: proposal.id,
            ..Default::default()
        }))
        .await;
    assert!(applied.is_ok(), "{applied:?}");
    let tasks: Vec<String> = board(&client, &project.id)
        .await
        .into_iter()
        .map(|(title, _)| title)
        .collect();
    assert_eq!(tasks, ["Log in"]);

    // A commit made since is what its next turn reads, and it says which.
    commit(repository.path(), "later.txt");
    send(&client, &session.id, "again").await;
    after_turns(&client, &session.id, 2).await;
    assert!(copy.join("later.txt").is_file());
    let commits: Vec<String> = listed(&client, &session.id)
        .await
        .code_commits
        .into_iter()
        .map(|at| at.commit)
        .collect();
    assert_eq!(commits, [head(repository.path())]);

    // It stays in plan mode, and archiving it removes its copy.
    assert!(!offers_plan(&client, &session.id).await);
    let full = update(&client, access(&session.id, pb::AccessLevel::Full)).await;
    assert!(full.is_err(), "{full:?}");
    let archived = update(
        &client,
        pb::UpdateSession {
            session_id: session.id.clone(),
            archived: Some(true),
            ..Default::default()
        },
    )
    .await;
    assert!(archived.is_ok(), "{archived:?}");
    assert!(!copy.exists());
}

#[tokio::test]
async fn a_planning_session_without_the_code_gets_no_copy_of_it() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let client = daemon(state.path(), Hosts::default()).await;
    let project = project(&client, repository.path()).await;

    let session = started(&client, planning(&project.id, false)).await;

    assert!(session.checkouts.is_empty(), "{session:?}");
    assert!(
        !Path::new(&session.working_directory)
            .join("README.md")
            .exists()
    );
}

#[tokio::test]
async fn plan_mode_holds_a_working_session_to_reading_from_its_next_turn_until_its_turned_off() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let client = daemon(state.path(), Hosts::default()).await;
    let project = project(&client, repository.path()).await;
    let mut request = start_in(&project.id);
    if let pb::request::Kind::StartSession(start) = &mut request {
        start.approval_mode = pb::ApprovalMode::AllowEverything.into();
        start.backend_config = r#"{"rules": [
            {"when": "one", "steps": [{"step": "write_file", "path": "a.txt", "contents": "one"}]},
            {"when": "two", "steps": [{"step": "write_file", "path": "a.txt", "contents": "two"}]},
            {"when": "three", "steps": [{"step": "write_file", "path": "a.txt", "contents": "three"}]}
        ]}"#
        .to_owned();
    }
    let session = started(&client, request).await;
    assert_eq!(session.access(), pb::AccessLevel::Full);
    assert!(offers_plan(&client, &session.id).await);
    send(&client, &session.id, "one").await;
    let before = notices(&after_turns(&client, &session.id, 1).await);
    let file = PathBuf::from(&listed(&client, &session.id).await.working_directory).join("a.txt");

    let planning = update(&client, access(&session.id, pb::AccessLevel::Plan)).await;
    assert_eq!(
        planning.map(|session| session.access()).ok(),
        Some(pb::AccessLevel::Plan)
    );
    send(&client, &session.id, "two").await;
    after_turns(&client, &session.id, 2).await;
    // Its own copy, with what it wrote before, is what it reads.
    assert_eq!(std::fs::read_to_string(&file).ok().as_deref(), Some("one"));

    let working = update(&client, access(&session.id, pb::AccessLevel::Full)).await;
    assert!(working.is_ok(), "{working:?}");
    send(&client, &session.id, "three").await;
    let events = after_turns(&client, &session.id, 3).await;
    assert_eq!(
        std::fs::read_to_string(&file).ok().as_deref(),
        Some("three")
    );
    // Each switch is said in the chat.
    assert_eq!(notices(&events), before + 2, "{events:#?}");
}

#[tokio::test]
async fn a_side_chat_stays_a_side_chat() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let client = daemon(state.path(), Hosts::default()).await;
    let project = project(&client, repository.path()).await;
    let session = started(&client, start_in(&project.id)).await;
    let forked = client
        .call(pb::request::Kind::ForkSession(pb::ForkSession {
            session_id: session.id.clone(),
            read_only: true,
            ..Default::default()
        }))
        .await;
    let Ok(pb::response::Kind::SessionForked(pb::SessionForked {
        session: Some(side),
    })) = forked
    else {
        panic!("{forked:?}")
    };

    assert_eq!(side.access(), pb::AccessLevel::SideChat);
    assert!(side.read_only);
    assert!(!offers_plan(&client, &side.id).await);
    for level in [pb::AccessLevel::Full, pb::AccessLevel::Plan] {
        let changed = update(&client, access(&side.id, level)).await;
        assert!(changed.is_err(), "{level:?}: {changed:?}");
    }
    assert_eq!(
        listed(&client, &side.id).await.access(),
        pb::AccessLevel::SideChat
    );
}

#[tokio::test]
async fn a_project_keeps_whether_planning_starts_with_the_code() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let client = daemon(state.path(), Hosts::default()).await;
    let project = project(&client, repository.path()).await;
    assert!(!project.plan_with_code);

    let projects = || async {
        match client
            .call(pb::request::Kind::ListProjects(pb::ListProjects {}))
            .await
        {
            Ok(pb::response::Kind::Projects(listed)) => listed.projects,
            other => panic!("{other:?}"),
        }
    };
    for with_code in [true, false] {
        let updated = client
            .call(pb::request::Kind::UpdateProject(pb::UpdateProject {
                project_id: project.id.clone(),
                plan_with_code: Some(with_code),
                ..Default::default()
            }))
            .await;
        assert!(updated.is_ok(), "{updated:?}");
        let kept: Vec<bool> = projects()
            .await
            .iter()
            .map(|project| project.plan_with_code)
            .collect();
        assert_eq!(kept, [with_code]);
    }
}
