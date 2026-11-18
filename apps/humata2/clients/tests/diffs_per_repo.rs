//! A spec gives each of a session's repositories its own base on the fake
//! (VIEWS C1): a repository the spec names is compared with its own base, one
//! the spec leaves out is compared with its default branch, and a repository
//! the session doesn't have, or one listed twice, is refused.

use std::sync::Arc;

use humata2_clients::model::{DiffBase, FileDiff, RepoBase, RepoDiff, RepoDiffSpec, SessionId};
use humata2_clients::{Daemon, DaemonError, FakeDaemon, fixture};
use tca::TestClock;
use tokio::runtime::Builder;

const SESSION: &str = "s-auth";

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn session() -> SessionId {
    SessionId::new(SESSION)
}

fn spec(repo: &str, base: RepoBase) -> RepoDiffSpec {
    RepoDiffSpec {
        repo: repo.to_owned(),
        base,
    }
}

fn branch(name: &str) -> RepoBase {
    RepoBase::Branch(name.to_owned())
}

fn file(path: &str, added: u32, removed: u32) -> FileDiff {
    FileDiff {
        path: path.to_owned(),
        added,
        removed,
        lines: Vec::new(),
    }
}

fn changes(repo: &str, files: Vec<FileDiff>) -> RepoDiff {
    RepoDiff {
        session: session(),
        repo: repo.to_owned(),
        base: "main".to_owned(),
        files,
        comments: Vec::new(),
    }
}

/// A session with changes in acme/api and acme/web, both against main.
fn daemon() -> FakeDaemon {
    let daemon = FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()));
    daemon.set_diffs(
        &session(),
        vec![
            changes(
                "acme/api",
                vec![file("src/client.rs", 4, 2), file("src/retry.rs", 2, 0)],
            ),
            changes("acme/web", vec![file("src/auth/SignIn.tsx", 3, 1)]),
        ],
    );
    daemon
}

fn read(daemon: &FakeDaemon, base: &DiffBase) -> Vec<RepoDiff> {
    match block_on(daemon.diffs(&session(), base)) {
        Ok(diffs) => diffs,
        Err(error) => panic!("the diffs are read: {error}"),
    }
}

fn of<'a>(diffs: &'a [RepoDiff], repo: &str) -> &'a RepoDiff {
    match diffs.iter().find(|diff| diff.repo == repo) {
        Some(diff) => diff,
        None => panic!("{repo} is in the diffs"),
    }
}

fn paths(diff: &RepoDiff) -> Vec<String> {
    diff.files.iter().map(|file| file.path.clone()).collect()
}

#[test]
fn each_repository_is_compared_with_its_own_base() {
    let daemon = daemon();
    let diffs = read(
        &daemon,
        &DiffBase::Spec(vec![
            spec("acme/api", branch("release/2.4")),
            spec("acme/web", branch("main")),
        ]),
    );
    let main = read(&daemon, &DiffBase::OriginMain);
    let api = of(&diffs, "acme/api");
    let web = of(&diffs, "acme/web");
    assert_eq!(api.base, "release/2.4");
    assert_eq!(web.base, "main");

    let mut api_paths = paths(api);
    api_paths.sort();
    assert_eq!(api_paths, ["docs/auth.md", "src/client.rs", "src/token.rs"]);
    assert_ne!(paths(api), paths(of(&main, "acme/api")));
    assert_eq!(paths(web), paths(of(&main, "acme/web")));
}

#[test]
fn a_repository_the_spec_leaves_out_is_compared_with_the_default_branch() {
    let daemon = daemon();
    let diffs = read(
        &daemon,
        &DiffBase::Spec(vec![spec("acme/api", branch("release/2.4"))]),
    );
    let main = read(&daemon, &DiffBase::OriginMain);
    let web = of(&diffs, "acme/web");
    assert_eq!(web.base, "main");
    assert_eq!(paths(web), paths(of(&main, "acme/web")));
}

#[test]
fn a_preset_still_compares_every_repository_with_main() {
    let daemon = daemon();
    let diffs = read(&daemon, &DiffBase::OriginMain);
    assert!(diffs.iter().all(|diff| diff.base == "main"));
    assert_eq!(paths(of(&diffs, "acme/api")), ["src/client.rs", "src/retry.rs"]);
}

#[test]
fn a_repository_the_session_does_not_have_is_refused() {
    let daemon = daemon();
    let result = block_on(daemon.diffs(
        &session(),
        &DiffBase::Spec(vec![spec("acme/infra", RepoBase::DefaultBranch)]),
    ));
    assert!(matches!(result, Err(DaemonError::Refused { .. })));
}

#[test]
fn a_repository_listed_twice_is_refused() {
    let daemon = daemon();
    let result = block_on(daemon.diffs(
        &session(),
        &DiffBase::Spec(vec![
            spec("acme/api", branch("main")),
            spec("acme/api", RepoBase::DefaultBranch),
        ]),
    ));
    assert!(matches!(result, Err(DaemonError::Refused { .. })));
}
