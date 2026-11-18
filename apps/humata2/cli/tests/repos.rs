//! `humata repos search|list|remove` (RL-5, RL-11): a search with no flags
//! leaves the scope to the daemon's saved one, flags give one call's scope,
//! every GitHub page is gathered, a problem still exits 0, and the library
//! commands send the repo as asked; forget-all asks first and scope set
//! saves only a scope that searches something.

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use pb::error::Kind as Refusal;
use pb::repo_scope::Kind;
use serde_json::json;
use support::{FakeConsole, FakeDaemon, run};

fn repo(owner: &str, name: &str) -> Option<pb::RepoRef> {
    Some(pb::RepoRef {
        owner: owner.into(),
        name: name.into(),
    })
}

fn found(owner: &str, name: &str) -> pb::FoundRepo {
    pb::FoundRepo {
        repo: repo(owner, name),
        description: format!("{name} of {owner}"),
        is_private: false,
    }
}

fn answer(github: Vec<pb::FoundRepo>, next: &str) -> pb::SearchReposResponse {
    pb::SearchReposResponse {
        library: vec![pb::LibraryRepo {
            repo: repo("acme", "api"),
            ..Default::default()
        }],
        github,
        page: Some(pb::PageResponse {
            next_page_token: next.into(),
        }),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_search_without_flags_names_no_scope() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::repos::SEARCH_REPOS, &answer(vec![], ""));
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "repos", "search", "led", "ger"]).await;
    assert_eq!(exit, exit::OK);
    let sent: pb::SearchReposRequest = daemon.only(rpc::repos::SEARCH_REPOS);
    assert_eq!(sent.query, "led ger");
    assert!(sent.scope.is_none());
}

#[tokio::test]
async fn owners_give_this_searchs_scope_without_the_login() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::repos::SEARCH_REPOS, &answer(vec![], ""));
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["repos", "search", "api", "--owner", "acme", "--owner", "bazelbuild"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::SearchReposRequest = daemon.only(rpc::repos::SEARCH_REPOS);
    let Some(Kind::Owners(owners)) = sent.scope.and_then(|scope| scope.kind) else {
        panic!("the scope should be owners");
    };
    assert_eq!(owners.owners, ["acme", "bazelbuild"]);
    assert!(owners.include_me);
}

#[tokio::test]
async fn all_of_github_and_your_repos_are_each_one_searchs_scope() {
    for (flag, expected) in [
        ("--all-of-github", Kind::AllOfGithub(true)),
        ("--your-repos", Kind::YourRepos(true)),
    ] {
        let daemon = FakeDaemon::new();
        daemon.reply(rpc::repos::SEARCH_REPOS, &answer(vec![], ""));
        let console = FakeConsole::pipe();
        assert_eq!(run(&daemon, &console, &["repos", "search", "api", flag]).await, exit::OK);
        let sent: pb::SearchReposRequest = daemon.only(rpc::repos::SEARCH_REPOS);
        assert_eq!(sent.scope.and_then(|scope| scope.kind), Some(expected), "{flag}");
    }
}

#[tokio::test]
async fn a_search_that_cannot_be_asked_is_a_usage_error_before_any_call() {
    for words in [
        &["repos", "search"][..],
        &["repos", "search", "  "][..],
        &["repos", "search", "api", "--all-of-github", "--owner", "acme"][..],
        &["repos", "search", "api", "--all-of-github", "--your-repos"][..],
        &["repos", "search", "api", "--owner", "acme/api"][..],
    ] {
        let daemon = FakeDaemon::new();
        let console = FakeConsole::pipe();
        assert_eq!(run(&daemon, &console, words).await, exit::USAGE, "{words:?}");
        assert!(!daemon.called(rpc::repos::SEARCH_REPOS), "{words:?}");
    }
}

#[tokio::test]
async fn every_github_page_is_gathered_in_order() {
    let daemon = FakeDaemon::new();
    daemon
        .reply(
            rpc::repos::SEARCH_REPOS,
            &answer(vec![found("acme", "one"), found("acme", "two")], "p2"),
        )
        .reply(rpc::repos::SEARCH_REPOS, &answer(vec![found("acme", "three")], ""));
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "repos", "search", "acme"]).await;
    assert_eq!(exit, exit::OK);
    let sent: Vec<pb::SearchReposRequest> = daemon.sent(rpc::repos::SEARCH_REPOS);
    let tokens: Vec<_> = sent
        .iter()
        .map(|request| request.page.clone().unwrap_or_default().page_token)
        .collect();
    assert_eq!(tokens, [String::new(), "p2".to_owned()]);
    let result = &console.json()["result"];
    let names: Vec<_> = result["github"]
        .as_array()
        .unwrap()
        .iter()
        .map(|found| found["repo"]["name"].clone())
        .collect();
    assert_eq!(names, [json!("one"), json!("two"), json!("three")]);
    assert_eq!(result["library"][0]["repo"]["name"], "api");
}

#[tokio::test]
async fn a_github_problem_still_exits_zero_with_the_library_answered() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::repos::SEARCH_REPOS,
        &pb::SearchReposResponse {
            problem: Some(pb::RepoSearchProblem {
                kind: pb::repo_search_problem::Kind::GhSignedOut as i32,
                message: "gh is signed out".into(),
                ..Default::default()
            }),
            ..answer(vec![], "")
        },
    );
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "repos", "search", "api"]).await;
    assert_eq!(exit, exit::OK);
    let result = &console.json()["result"];
    assert_eq!(result["problem"]["kind"], "gh_signed_out");
    assert_eq!(result["library"][0]["repo"]["name"], "api");
}

#[tokio::test]
async fn list_gives_the_library_in_the_order_the_daemon_sent_it() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::repos::LIST_REPO_LIBRARY,
        &pb::ListRepoLibraryResponse {
            repos: vec![
                pb::LibraryRepo {
                    repo: repo("acme", "newer"),
                    ..Default::default()
                },
                pb::LibraryRepo {
                    repo: repo("acme", "older"),
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
    );
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["--json", "repos", "list"]).await, exit::OK);
    let names: Vec<_> = console.json()["result"]["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|library| library["repo"]["name"].clone())
        .collect();
    assert_eq!(names, [json!("newer"), json!("older")]);
}

#[tokio::test]
async fn remove_sends_the_repo() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::repos::REMOVE_FROM_REPO_LIBRARY,
        &pb::ListRepoLibraryResponse::default(),
    );
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["repos", "remove", "acme/api"]).await;
    assert_eq!(exit, exit::OK);
    let sent: pb::RemoveFromRepoLibraryRequest = daemon.only(rpc::repos::REMOVE_FROM_REPO_LIBRARY);
    assert_eq!(sent.repo, repo("acme", "api"));
}

#[tokio::test]
async fn a_repo_that_is_not_owner_slash_name_is_a_usage_error_before_any_call() {
    for bad in ["api", "acme/", "/api", "a/b/c"] {
        let daemon = FakeDaemon::new();
        let console = FakeConsole::pipe();
        assert_eq!(run(&daemon, &console, &["repos", "remove", bad]).await, exit::USAGE, "{bad}");
        assert!(!daemon.called(rpc::repos::REMOVE_FROM_REPO_LIBRARY), "{bad}");
    }
}

#[tokio::test]
async fn a_remove_the_daemon_refuses_exits_with_that_kinds_code() {
    for (kind, code) in [
        (Refusal::UserOnly(Default::default()), exit::NEEDS_USER),
        (Refusal::Denied(Default::default()), exit::DENIED),
    ] {
        let daemon = FakeDaemon::new();
        daemon.refuse(rpc::repos::REMOVE_FROM_REPO_LIBRARY, kind);
        let console = FakeConsole::pipe();
        let exit = run(&daemon, &console, &["--json", "repos", "remove", "acme/api"]).await;
        assert_eq!(exit, code);
        assert_eq!(console.json()["ok"], false);
    }
}

fn library_of(count: usize) -> pb::ListRepoLibraryResponse {
    pb::ListRepoLibraryResponse {
        repos: (0..count)
            .map(|n| pb::LibraryRepo {
                repo: repo("acme", &format!("repo{n}")),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

fn forgettable() -> FakeDaemon {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::repos::LIST_REPO_LIBRARY, &library_of(2));
    daemon.reply(
        rpc::repos::FORGET_REPO_LIBRARY,
        &pb::ForgetRepoLibraryResponse { forgotten: 2 },
    );
    daemon
}

#[tokio::test]
async fn forget_all_in_a_pipe_without_yes_forgets_nothing() {
    let daemon = forgettable();
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["repos", "forget-all"]).await;
    assert_eq!(exit, exit::USAGE);
    assert!(!daemon.called(rpc::repos::FORGET_REPO_LIBRARY));
}

#[tokio::test]
async fn forget_all_asks_first_and_a_no_forgets_nothing() {
    let daemon = forgettable();
    let console = FakeConsole::terminal().answering(false);
    let exit = run(&daemon, &console, &["repos", "forget-all"]).await;
    assert_ne!(exit, exit::OK);
    assert!(!daemon.called(rpc::repos::FORGET_REPO_LIBRARY));
}

#[tokio::test]
async fn forget_all_forgets_after_a_yes_or_with_the_yes_flag() {
    for (console, words) in [
        (FakeConsole::terminal().answering(true), &["repos", "forget-all"][..]),
        (FakeConsole::pipe(), &["--yes", "repos", "forget-all"][..]),
    ] {
        let daemon = forgettable();
        let exit = run(&daemon, &console, words).await;
        assert_eq!(exit, exit::OK, "{words:?}");
        assert!(daemon.called(rpc::repos::FORGET_REPO_LIBRARY), "{words:?}");
    }
}

#[tokio::test]
async fn forget_all_with_dry_run_forgets_nothing() {
    let daemon = forgettable();
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "--dry-run", "repos", "forget-all"]).await;
    assert_eq!(exit, exit::OK);
    assert!(!daemon.called(rpc::repos::FORGET_REPO_LIBRARY));
}

#[tokio::test]
async fn scope_show_reads_the_saved_scope() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::repos::GET_REPO_SCOPE,
        &pb::RepoScope {
            kind: Some(Kind::YourRepos(true)),
        },
    );
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "repos", "scope", "show"]).await;
    assert_eq!(exit, exit::OK);
    assert!(daemon.called(rpc::repos::GET_REPO_SCOPE));
    assert!(!daemon.called(rpc::repos::UPDATE_REPO_SCOPE));
}

#[tokio::test]
async fn scope_set_saves_the_owners_with_or_without_your_repos() {
    for (extra, include_me) in [(&[][..], true), (&["--no-include-me"][..], false)] {
        let daemon = FakeDaemon::new();
        daemon.reply(rpc::repos::UPDATE_REPO_SCOPE, &pb::RepoScope::default());
        let console = FakeConsole::pipe();
        let mut words = vec!["repos", "scope", "set", "--owner", "acme", "--owner", "bazelbuild"];
        words.extend_from_slice(extra);
        assert_eq!(run(&daemon, &console, &words).await, exit::OK, "{words:?}");
        let sent: pb::UpdateRepoScopeRequest = daemon.only(rpc::repos::UPDATE_REPO_SCOPE);
        let Some(Kind::Owners(owners)) = sent.scope.and_then(|scope| scope.kind) else {
            panic!("the saved scope should be owners: {words:?}");
        };
        assert_eq!(owners.owners, ["acme", "bazelbuild"]);
        assert_eq!(owners.include_me, include_me);
    }
}

#[tokio::test]
async fn scope_set_without_owners_saves_your_repos() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::repos::UPDATE_REPO_SCOPE, &pb::RepoScope::default());
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["repos", "scope", "set"]).await, exit::OK);
    let sent: pb::UpdateRepoScopeRequest = daemon.only(rpc::repos::UPDATE_REPO_SCOPE);
    assert_eq!(sent.scope.and_then(|scope| scope.kind), Some(Kind::YourRepos(true)));
}

#[tokio::test]
async fn a_scope_that_searches_nothing_is_a_usage_error_before_any_call() {
    for words in [
        &["repos", "scope", "set", "--no-include-me"][..],
        &["repos", "scope", "set", "--owner", "acme/api"][..],
        &["repos", "scope", "set", "--owner", " "][..],
    ] {
        let daemon = FakeDaemon::new();
        let console = FakeConsole::pipe();
        assert_eq!(run(&daemon, &console, words).await, exit::USAGE, "{words:?}");
        assert!(!daemon.called(rpc::repos::UPDATE_REPO_SCOPE), "{words:?}");
    }
}
