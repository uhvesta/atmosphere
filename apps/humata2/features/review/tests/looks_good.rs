//! D852: Looks good ticks every file the panel shows, a file whose diff
//! changes stops being looked, a layer reports when a looked file changed,
//! and the "Since I last looked" switch goes to the bookmark's base and back.
use std::sync::Arc;

use humata2_clients::fixture;
use humata2_clients::model::{
    DiffBase, DiffLine, FileDiff, LineKind, RepoDiff, SessionId, Status,
};
use humata2_clients::{FakeDaemon, Workspace};
use humata2_review::looks::{FileLook, LayerLook, any_look, file_look, is_looked, layer_look, looked_count};
use humata2_review::{Review, ReviewAction as A, ReviewState};
use tca::{TestClock, TestStore};

fn session() -> SessionId {
    SessionId::new("ledger")
}

fn workspace() -> Workspace {
    let mut workspace = fixture::design();
    for each in &mut workspace.sessions {
        if each.id == session() {
            each.status = Status::Idle;
        }
    }
    workspace
}

fn file(path: &str, text: &str) -> FileDiff {
    FileDiff {
        path: path.to_owned(),
        added: 1,
        lines: vec![
            DiffLine {
                kind: LineKind::Hunk,
                old: None,
                new: None,
                text: "@@ -0,0 +1 @@".to_owned(),
            },
            DiffLine {
                kind: LineKind::Added,
                old: None,
                new: Some(1),
                text: text.to_owned(),
            },
        ],
        ..FileDiff::default()
    }
}

fn change(repo: &str, files: Vec<FileDiff>) -> RepoDiff {
    RepoDiff {
        session: session(),
        repo: repo.to_owned(),
        base: "origin/main".to_owned(),
        files,
        comments: Vec::new(),
    }
}

struct Rig {
    store: TestStore<Review>,
    daemon: FakeDaemon,
}

impl Rig {
    fn act(&mut self, action: A) {
        self.store.send(action, |_| {});
        for _ in 0..6 {
            self.store.skip_received();
        }
    }

    fn looked(&self, repo: &str, path: &str) -> bool {
        let state = self.store.state();
        let diffs = state.fetch[&session()].diffs.clone().unwrap_or_default();
        diffs
            .iter()
            .filter(|diff| diff.repo == repo)
            .flat_map(|diff| &diff.files)
            .filter(|each| each.path == path)
            .any(|each| is_looked(state, &session(), repo, path, each))
    }

    fn count(&self) -> usize {
        let state = self.store.state();
        let diffs = state.fetch[&session()].diffs.clone().unwrap_or_default();
        looked_count(state, &session(), &diffs)
    }

    fn change_again(&mut self, diffs: Vec<RepoDiff>) {
        self.daemon.set_diffs(&session(), diffs);
        self.act(A::Retry);
    }
}

fn rig(diffs: Vec<RepoDiff>, layer: Option<&str>) -> Rig {
    let clock = Arc::new(TestClock::new());
    let daemon = FakeDaemon::new(workspace(), clock.clone());
    daemon.set_diffs(&session(), diffs);
    let mut shown = workspace();
    shown.diffs.clear();
    let mut state = ReviewState {
        workspace: Arc::new(shown),
        ..ReviewState::default()
    };
    if let Some(title) = layer {
        state.layer.insert(session(), Some(title.to_owned()));
    }
    let store = TestStore::new(state, Review::new(Arc::new(daemon.clone()), clock)).non_exhaustive();
    let mut rig = Rig { store, daemon };
    rig.act(A::Open(session()));
    rig
}

fn two_repos() -> Vec<RepoDiff> {
    vec![
        change("api", vec![file("src/a.rs", "one"), file("src/b.rs", "two")]),
        change("web", vec![file("src/c.ts", "three")]),
    ]
}

#[test]
fn looks_good_ticks_every_file_shown_and_moves_the_bookmark() {
    let mut rig = rig(two_repos(), None);
    assert_eq!(rig.count(), 0);
    rig.act(A::LooksGood);
    assert_eq!(rig.count(), 3);
    assert!(rig.looked("web", "src/c.ts"));
    assert!(
        rig.daemon
            .calls()
            .iter()
            .any(|call| call == "mark reviewed ledger")
    );
}

#[test]
fn a_file_whose_diff_changes_is_no_longer_looked() {
    let mut rig = rig(two_repos(), None);
    rig.act(A::LooksGood);
    rig.change_again(vec![
        change("api", vec![file("src/a.rs", "one"), file("src/b.rs", "two, edited")]),
        change("web", vec![file("src/c.ts", "three")]),
    ]);
    assert!(rig.looked("api", "src/a.rs"));
    assert!(!rig.looked("api", "src/b.rs"));
    assert_eq!(rig.count(), 2);
}

#[test]
fn clear_looks_and_toggle_work_on_the_files_shown() {
    let mut rig = rig(two_repos(), None);
    rig.act(A::ToggleLooked {
        repo: "api".to_owned(),
        path: "src/a.rs".to_owned(),
    });
    assert!(rig.looked("api", "src/a.rs"));
    assert_eq!(rig.count(), 1);
    rig.act(A::ToggleLooked {
        repo: "api".to_owned(),
        path: "src/a.rs".to_owned(),
    });
    assert_eq!(rig.count(), 0);

    rig.act(A::LooksGood);
    assert_eq!(rig.count(), 3);
    rig.act(A::ClearLooks);
    assert_eq!(rig.count(), 0);
}

#[test]
fn a_layer_reports_changed_since_looked_when_a_looked_file_changes() {
    let mut rig = rig(two_repos(), Some("schema"));
    assert_eq!(layer_look(rig.store.state(), &session(), "schema"), LayerLook::None);
    rig.act(A::LooksGood);
    assert_eq!(layer_look(rig.store.state(), &session(), "schema"), LayerLook::LooksGood);
    assert_eq!(layer_look(rig.store.state(), &session(), "client"), LayerLook::None);

    rig.change_again(vec![
        change("api", vec![file("src/a.rs", "one, edited"), file("src/b.rs", "two")]),
        change("web", vec![file("src/c.ts", "three")]),
    ]);
    assert_eq!(
        layer_look(rig.store.state(), &session(), "schema"),
        LayerLook::ChangedSinceLooked
    );

    rig.act(A::ClearLooks);
    assert_eq!(layer_look(rig.store.state(), &session(), "schema"), LayerLook::None);
}

#[test]
fn the_since_looked_switch_goes_to_the_bookmark_base_and_back() {
    let mut rig = rig(two_repos(), None);
    rig.act(A::PickBase(DiffBase::UncommittedOnly));
    rig.act(A::SinceLooked(true));
    assert_eq!(
        rig.store.state().targets_of(&session()).preset,
        DiffBase::SinceLastReviewed
    );
    rig.act(A::SinceLooked(false));
    assert_eq!(
        rig.store.state().targets_of(&session()).preset,
        DiffBase::UncommittedOnly
    );
}

#[test]
fn a_file_is_never_looked_looked_or_changed_since() {
    let mut rig = rig(two_repos(), None);
    let look_of = |rig: &Rig, path: &str| {
        let state = rig.store.state();
        let diffs = state.fetch[&session()].diffs.clone().unwrap_or_default();
        let (repo, file) = diffs
            .iter()
            .flat_map(|diff| diff.files.iter().map(move |file| (&diff.repo, file)))
            .find(|(_, file)| file.path == path)
            .expect("the file is read");
        file_look(state, &session(), repo, path, file)
    };
    assert_eq!(look_of(&rig, "src/b.rs"), FileLook::Never);
    assert!(!any_look(rig.store.state(), &session()));
    rig.act(A::LooksGood);
    assert_eq!(look_of(&rig, "src/b.rs"), FileLook::Looked);
    assert!(any_look(rig.store.state(), &session()));
    rig.change_again(vec![
        change("api", vec![file("src/a.rs", "one"), file("src/b.rs", "two, edited")]),
        change("web", vec![file("src/c.ts", "three")]),
    ]);
    assert_eq!(look_of(&rig, "src/b.rs"), FileLook::Changed);
    assert_eq!(look_of(&rig, "src/a.rs"), FileLook::Looked);
    rig.act(A::ClearLooks);
    assert_eq!(look_of(&rig, "src/b.rs"), FileLook::Never);
}
