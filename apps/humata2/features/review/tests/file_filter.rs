//! RV-6: the file filter keeps matching files and the repos that have them,
//! and is kept per session.
use std::sync::Arc;

use humata2_clients::fixture;
use humata2_clients::model::{FileDiff, RepoDiff, SessionId};
use humata2_review::file_filter::{matches, shown_for};
use humata2_review::{Review, ReviewAction as A, ReviewState};
use tca::Reducer;

fn files(session: &str, repo: &str, paths: &[String]) -> RepoDiff {
    RepoDiff {
        session: SessionId::new(session),
        repo: repo.to_owned(),
        base: "origin/main".to_owned(),
        files: paths
            .iter()
            .map(|path| FileDiff { path: path.clone(), added: 1, ..FileDiff::default() })
            .collect(),
        comments: Vec::new(),
    }
}

fn state() -> ReviewState {
    let mut workspace = fixture::design();
    workspace.diffs.clear();
    let many: Vec<String> = (0..21).map(|n| format!("src/file_{n}.rs")).collect();
    workspace.diffs.push(files("ledger", "api", &many));
    workspace.diffs.push(files("ledger", "web", &["docs/Readme.md".to_owned()]));
    workspace.diffs.push(files("partial-brin", "api", &["src/a.rs".to_owned()]));
    ReviewState { workspace: Arc::new(workspace), ..ReviewState::default() }
}

fn filter(state: &mut ReviewState, session: &str, query: &str) {
    let reducer = Review::new(
        Arc::new(humata2_clients::FakeDaemon::new(
            (*state.workspace).clone(),
            Arc::new(tca::TestClock::new()),
        )),
        Arc::new(tca::TestClock::new()),
    );
    let _ = reducer.reduce(state, A::FilterFiles(SessionId::new(session), query.to_owned()));
}

#[test]
fn filtering_keeps_only_matching_files_and_their_repos() {
    let mut state = state();
    let ledger = SessionId::new("ledger");
    assert_eq!(state.visible_files(&ledger).len(), 22);
    filter(&mut state, "ledger", "READ");
    assert_eq!(
        state.visible_files(&ledger),
        vec![("web".to_owned(), "docs/Readme.md".to_owned())]
    );
    filter(&mut state, "ledger", "");
    assert_eq!(state.visible_files(&ledger).len(), 22);
}

#[test]
fn the_filter_is_kept_per_session() {
    let mut state = state();
    filter(&mut state, "ledger", "file_3");
    assert_eq!(state.file_filter(&SessionId::new("ledger")), "file_3");
    assert_eq!(state.file_filter(&SessionId::new("partial-brin")), "");
    // A session with 20 files or fewer is never filtered.
    filter(&mut state, "partial-brin", "zzz");
    assert_eq!(state.visible_files(&SessionId::new("partial-brin")).len(), 1);
}

    #[test]
    fn it_matches_the_name_ignoring_case() {
        assert!(matches("src/Client.rs", "client"));
        assert!(!matches("src/server.rs", "client"));
    }

    #[test]
    fn it_matches_the_folder() {
        assert!(matches("src/net/client.rs", "NET"));
        assert!(matches("src/net/client.rs", "net/cl"));
    }

    #[test]
    fn an_empty_query_matches_everything() {
        assert!(matches("a.rs", ""));
        assert!(matches("a.rs", "   "));
    }

    #[test]
    fn the_field_needs_more_than_twenty_files() {
        assert!(!shown_for(20));
        assert!(shown_for(21));
    }

    #[test]
    fn a_pull_request_review_with_more_than_twenty_files_filters_like_plain_diff() {
        let mut workspace = fixture::design();
        let pr_session = SessionId::new("pr-review");
        if let Some(session) = workspace.sessions.first_mut() {
            session.id = pr_session.clone();
            session.pull_request_review = true;
        }
        workspace.diffs.clear();
        let api_files: Vec<String> = (0..21).map(|n| format!("src/handler_{n}.rs")).collect();
        workspace.diffs.push(files("pr-review", "acme/api", &api_files));
        workspace
            .diffs
            .push(files("pr-review", "acme/web", &["ui/checkout.tsx".to_owned()]));
        let mut state = ReviewState {
            workspace: Arc::new(workspace),
            ..ReviewState::default()
        };
        assert!(state.file_filter_shown(&pr_session));
        filter(&mut state, "pr-review", "checkout");
        assert!(state.file_visible(&pr_session, "ui/checkout.tsx"));
        assert!(!state.file_visible(&pr_session, "src/handler_0.rs"));
        assert_eq!(
            state.visible_files(&pr_session),
            vec![("acme/web".to_owned(), "ui/checkout.tsx".to_owned())]
        );
    }
