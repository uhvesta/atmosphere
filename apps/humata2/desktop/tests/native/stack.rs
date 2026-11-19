//! D800, DS-60, DS-62, DS-66: a stack review's layer list in the Diff panel.
//!
//! These cases need the Diff panel to draw the list from the open plan's
//! stack (the STACKVIEW T8 hook-up); they fail until it does.
use super::*;
use humata2_clients::model::{RepoBase, RepoDiffSpec};
use humata2_clients::views::{LayerRepoView, LayerView, PlanView, StackView, ViewContent};

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

fn part(repo: &str, base: RepoBase, pull_request: &str, files: u32) -> LayerRepoView {
    LayerRepoView {
        repo: repo.to_owned(),
        spec: RepoDiffSpec { repo: repo.to_owned(), base },
        pull_request: pull_request.to_owned(),
        files,
        drift: None,
    }
}

fn layer(title: &str, purpose: &str, repos: Vec<LayerRepoView>) -> LayerView {
    LayerView {
        title: title.to_owned(),
        purpose: purpose.to_owned(),
        repos,
        ..LayerView::default()
    }
}

/// The three-layer example: 1 "codec" in api and web with a PR each, 2
/// "journal" in api only, 3 "Not committed yet", a worktree layer in api.
fn stack() -> StackView {
    let range = |base: &str, head: &str| RepoBase::Range {
        base: base.to_owned(),
        head: head.to_owned(),
    };
    StackView {
        layers: vec![
            layer(
                "codec",
                "The wire format",
                vec![
                    part("acme/api", range("a0", "a1"), "acme/api#412", 3),
                    part("acme/web", range("w0", "w1"), "acme/web#88", 2),
                ],
            ),
            layer(
                "journal",
                "Write what the codec reads",
                vec![part("acme/api", range("a1", "a2"), "acme/api#413", 4)],
            ),
            layer(
                "Not committed yet",
                "",
                vec![part(
                    "acme/api",
                    RepoBase::Overlay { base: "a2".to_owned(), paths: vec![".".to_owned()] },
                    "",
                    1,
                )],
            ),
        ],
        whole: Vec::new(),
    }
}

/// Ledger's Diff panel showing the stack at Whole stack.
fn stack_app() -> App {
    let mut workspace = fixture::design_session(Status::Working);
    for diff in &mut workspace.diffs {
        diff.repo = format!("acme/{}", diff.repo);
    }
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Journal);
    app.fakes.views.offer_view(
        &ledger(),
        "Review plan: the stack",
        "Three layers",
        "Claude Code",
        ViewContent::ReviewPlan(PlanView {
            stack: Some(stack()),
            ..PlanView::default()
        }),
    );
    app.settle();
    app.click("offer-open");
    app
}

fn press(app: &mut App, key: &str) {
    app.cx
        .update_window(app.window, |_, window, cx| window.press(key, cx))
        .expect("window stays open");
    app.settle();
}

/// Whether `inner` is drawn inside `outer`.
fn inside(app: &mut App, inner: &'static str, outer: &'static str) -> bool {
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let inner = window.find(inner).bounds();
            let outer = window.find(outer).bounds();
            outer.contains(&inner.origin)
        })
        .expect("window stays open")
}

#[test]
fn a_stack_draws_whole_stack_then_its_layers_top_down() {
    let mut app = stack_app();
    assert!(app.contains("stack-list"));
    for row in 0..4 {
        assert!(app.contains(&format!("stack-row-{row}")), "row {row}");
    }
    assert!(!app.contains("stack-row-4"), "Whole stack and three layers");
    // Whole stack is picked on arrival; every row says what it touches.
    assert!(inside(&mut app, "stack-row-picked", "stack-row-0"));
    for row in 0..4 {
        assert!(app.contains(&format!("stack-row-note-{row}")), "row {row} names its repos");
    }
}

#[test]
fn the_next_layer_key_picks_the_bottom_layer_from_whole_stack() {
    let mut app = stack_app();
    // The layer keys work in the Diff panel (D804): Whole stack is clicked
    // there first.
    app.click("stack-row-0");
    assert!(inside(&mut app, "stack-row-picked", "stack-row-0"));
    press(&mut app, "alt-cmd-]");
    assert!(inside(&mut app, "stack-row-picked", "stack-row-3"));
    press(&mut app, "alt-cmd-[");
    assert!(inside(&mut app, "stack-row-picked", "stack-row-0"));
}

#[test]
fn clicking_a_layer_picks_it() {
    let mut app = stack_app();
    app.click("stack-row-2");
    assert!(inside(&mut app, "stack-row-picked", "stack-row-2"));
}

#[test]
fn a_stack_without_notes_draws_no_review_chrome() {
    let mut app = stack_app();
    for id in [
        "change-note-0",
        "change-ask-hunk-0",
        "change-ask-file acme/api/src/client.rs",
        "diff-mark-reviewed",
    ] {
        assert!(!app.contains(id), "{id} is review chrome");
    }
}

/// D852: a layer says "Changed since you looked" once a file it showed when
/// Looks good was pressed changes.
#[test]
fn a_layer_row_says_changed_since_you_looked_after_its_file_changes() {
    use humata2_clients::model::LineKind;
    use humata2_review::ReviewAction as R;
    let mut app = stack_app();
    // Row 1 is the top layer, "Not committed yet", which shows api's files.
    app.click("stack-row-1");
    app.click("diff-looks-good");
    assert!(app.contains("stack-row-look-1"), "the layer reads Looks good");
    let mut diffs: Vec<_> = app.ui.read(|state| {
        state.workspace.diffs.iter().filter(|diff| diff.session == ledger()).cloned().collect()
    });
    let file = diffs
        .iter_mut()
        .flat_map(|diff| diff.files.iter_mut())
        .next()
        .expect("a file in the layer");
    file.lines.iter_mut().find(|line| line.kind == LineKind::Added).unwrap().text =
        "reworded after the look".to_owned();
    app.fakes.daemon.set_diffs(&ledger(), diffs);
    app.send(AppAction::Review(R::Retry));
    assert!(app.contains("stack-row-look-1"));
    let look = app.ui.read(|state| {
        humata2_review::looks::layer_look(&state.review, &ledger(), "Not committed yet")
    });
    assert_eq!(look, humata2_review::looks::LayerLook::ChangedSinceLooked);
}

fn reword_first_file(app: &mut App) {
    use humata2_clients::model::LineKind;
    let mut diffs: Vec<_> = app.ui.read(|state| {
        state.workspace.diffs.iter().filter(|diff| diff.session == ledger()).cloned().collect()
    });
    let file = diffs
        .iter_mut()
        .flat_map(|diff| diff.files.iter_mut())
        .next()
        .expect("a file in the layer");
    file.lines.iter_mut().find(|line| line.kind == LineKind::Added).unwrap().text =
        "reworded after the look".to_owned();
    app.fakes.daemon.set_diffs(&ledger(), diffs);
    app.send(AppAction::Review(humata2_review::ReviewAction::Retry));
}

/// UXSIMPLE 3-2, 4-1 and 4-3: a layer that looks good, one that changed after
/// it, and the end state where every layer looks good.
#[test]
fn looks_good_frames_for_a_layer_a_changed_layer_and_the_whole_stack() {
    let mut app = stack_app();
    app.click("stack-row-1");
    // Sent, not clicked, so the pointer isn't left hovering the button.
    app.send(AppAction::Review(humata2_review::ReviewAction::LooksGood));
    app.snapshot("looks-good-layer");
    reword_first_file(&mut app);
    app.snapshot("looks-good-changed");
    // Every layer looks good, the whole stack too.
    for row in ["stack-row-2", "stack-row-3", "stack-row-1"] {
        app.click(row);
        app.send(AppAction::Review(humata2_review::ReviewAction::LooksGood));
    }
    app.click("stack-row-0");
    app.send(AppAction::Review(humata2_review::ReviewAction::LooksGood));
    app.snapshot("looks-good-all");
}
