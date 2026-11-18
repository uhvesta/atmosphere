//! VIEWS: a session over two repositories shows each against its own base.
//! Ledger has a `release/2.4` branch that `main` has moved on from; picking
//! it for ledger changes ledger's files and leaves api's as they were. A
//! real daemon, real clones and the real client.
use std::path::Path;
use std::process::Command;

use humata2_app::{AppAction, Route};
use humata2_clients::model::{Agent, RepoBase, RepoDiff, SessionId, SessionOptions};
use humata2_review::ReviewAction;
use humata2_session::{Panel, SessionAction};

use super::review::{clone_of, write};
use super::world::World;
use super::{App, agent_has, runtime};

const LEDGER: &str = "owner/ledger";
const API: &str = "owner/api";
const RELEASE: &str = "release/2.4";

fn git(args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Cuts `release/2.4` from the repository's first commit on its remote, then
/// moves `main` on with `main-only.md`, so a session branched from `main`
/// differs from `release/2.4` by one file more than it differs from `main`.
fn cut_release(root: &Path, name: &str) {
    let bare = root.join("remotes").join("owner").join(format!("{name}.git"));
    let work = root.join(format!("scratch-owner-{name}"));
    let (bare, work_text) = (bare.display().to_string(), work.display().to_string());
    git(&["-C", &work_text, "branch", RELEASE]);
    git(&["-C", &work_text, "push", &bare, RELEASE]);
    std::fs::write(work.join("main-only.md"), "main only\n").expect("write main-only.md");
    git(&["-C", &work_text, "add", "."]);
    git(&["-C", &work_text, "commit", "-m", "main moves on"]);
    git(&["-C", &work_text, "push", &bare, "main"]);
}

fn send(app: &mut App, action: AppAction) {
    app.cx.update(|cx| app.ui.send(action, cx));
    app.settle();
}

/// The session's changed paths in `repo`, sorted, once the app holds them.
fn paths(app: &App, session: &SessionId, repo: &str) -> Option<Vec<String>> {
    app.ui.read(|state| {
        let diff: &RepoDiff = state
            .workspace
            .diffs
            .iter()
            .find(|diff| &diff.session == session && diff.repo == repo)?;
        let mut paths: Vec<String> = diff.files.iter().map(|file| file.path.clone()).collect();
        paths.sort();
        Some(paths)
    })
}

#[test]
fn a_repository_picked_onto_a_release_branch_changes_and_its_neighbour_does_not() {
    let runtime = runtime();
    let (world, clients, session) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let mac = World::this_mac(&clients)
            .await
            .expect("the workspace has This Mac");
        let repos = vec![
            world.repo("owner", "ledger").expect("make ledger"),
            world.repo("owner", "api").expect("make api"),
        ];
        let root = world
            .channels_folder()
            .parent()
            .expect("the channels folder has a parent")
            .to_owned();
        cut_release(&root, "ledger");
        let channel = clients
            .daemon
            .create_channel("payments", repos.clone(), vec![mac.clone()])
            .await
            .expect("create the channel");
        let session = clients
            .daemon
            .create_session(
                &channel,
                Agent::Claude,
                &mac,
                "Ledger release fix",
                SessionOptions {
                    repos,
                    first_message: "hello agent".to_owned(),
                    ..SessionOptions::default()
                },
            )
            .await
            .expect("create the session");
        agent_has(&world, "hello agent").await;
        (world, clients, session)
    });
    let channels = world.channels_folder();
    write(&clone_of(&channels, "ledger"), "agent.md", "ledger change\n");
    write(&clone_of(&channels, "api"), "agent.md", "api change\n");

    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui.read(|state| state.workspace.session(&session).is_some())
    });
    send(&mut app, AppAction::Navigate(Route::Session(session.clone())));
    send(&mut app, AppAction::Session(SessionAction::SetPanel(Some(Panel::Changes))));

    // Both repositories start on the default branch.
    let only_agent = Some(vec!["agent.md".to_owned()]);
    app.wait_until("both repositories against main", |app| {
        paths(app, &session, LEDGER) == only_agent && paths(app, &session, API) == only_agent
    });
    assert!(!app.ui.read(|state| state.review.compares_per_repo(&session)));

    // Ledger's menu lists the branches its clone fetched.
    send(&mut app, AppAction::Review(ReviewAction::ToggleRepoBase { repo: LEDGER.to_owned() }));
    app.wait_until("ledger's branches", |app| {
        app.ui.read(|state| {
            matches!(
                state.review.repo_picker.as_ref().and_then(|picker| picker.bases.as_ref()),
                Some(Ok(bases)) if bases.branches.iter().any(|branch| branch == RELEASE)
            )
        })
    });

    // Picking release/2.4 changes ledger's files, not api's.
    send(
        &mut app,
        AppAction::Review(ReviewAction::PickRepoBase {
            session: session.clone(),
            repo: LEDGER.to_owned(),
            base: RepoBase::Branch(RELEASE.to_owned()),
        }),
    );
    app.wait_until("ledger against release/2.4", |app| {
        paths(app, &session, LEDGER) == Some(vec!["agent.md".to_owned(), "main-only.md".to_owned()])
    });
    assert_eq!(paths(&app, &session, API), only_agent, "api keeps main");
    app.ui.read(|state| {
        assert!(state.review.repo_picker.is_none(), "a pick closes the menu");
        assert!(state.review.compares_per_repo(&session), "the header reads per repo");
        let targets = state.review.targets_of(&session);
        assert_eq!(targets.of(LEDGER), RepoBase::Branch(RELEASE.to_owned()));
        assert_eq!(targets.of(API), RepoBase::DefaultBranch);
    });
}
