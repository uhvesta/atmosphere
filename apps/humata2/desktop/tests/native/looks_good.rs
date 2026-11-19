//! D852: Looks good ticks every file the Diff panel shows, a file whose diff
//! changes loses its tick, a file's circle toggles it, and the header switch
//! lists only the files changed since the last look.
use super::*;
use humata2_clients::model::{DiffBase, LineKind, RepoDiff};
use humata2_review::ReviewAction as R;
use humata2_review::looks::{file_count, is_looked, looked_count};

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

fn plain() -> App {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    app
}

/// The session's diffs as the panel shows them.
fn shown(app: &App) -> Vec<RepoDiff> {
    app.ui.read(|state| {
        state
            .workspace
            .diffs
            .iter()
            .filter(|diff| diff.session == ledger())
            .cloned()
            .collect()
    })
}

fn looked(app: &App) -> usize {
    let diffs = shown(app);
    app.ui
        .read(|state| looked_count(&state.review, &ledger(), &diffs))
}

fn file_looked(app: &App, repo: &str, path: &str) -> bool {
    let diffs = shown(app);
    let file = diffs
        .iter()
        .filter(|diff| diff.repo == repo)
        .flat_map(|diff| &diff.files)
        .find(|file| file.path == path)
        .expect("the file is in the diff");
    app.ui
        .read(|state| is_looked(&state.review, &ledger(), repo, path, file))
}

fn first_file(app: &App) -> (String, String) {
    let diffs = shown(app);
    diffs
        .iter()
        .flat_map(|diff| diff.files.iter().map(move |file| (diff, file)))
        .find(|(_, file)| file.lines.iter().any(|line| line.kind == LineKind::Added))
        .map(|(diff, file)| (diff.repo.clone(), file.path.clone()))
        .expect("a file with an added line")
}

/// The session's changes with one added line of `repo`/`path` reworded.
fn reword(app: &App, repo: &str, path: &str) -> Vec<RepoDiff> {
    let mut diffs = shown(app);
    let file = diffs
        .iter_mut()
        .filter(|diff| diff.repo == repo)
        .flat_map(|diff| diff.files.iter_mut())
        .find(|file| file.path == path)
        .expect("the file is in the diff");
    file.lines
        .iter_mut()
        .find(|line| line.kind == LineKind::Added)
        .expect("an added line")
        .text = "reworded after the look".to_owned();
    diffs
}

fn tree(app: &mut App) -> String {
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| view::capture(window))
        .expect("window")
        .expect("view")
}

#[test]
fn looks_good_ticks_every_file_and_the_button_flips_to_clear() {
    let mut app = plain();
    let total = file_count(&shown(&app));
    assert!(total > 1, "the fixture has several files");
    assert_eq!(looked(&app), 0);
    assert!(!tree(&mut app).contains("✓ Looks good"));
    app.click("diff-looks-good");
    assert_eq!(looked(&app), total);
    assert!(tree(&mut app).contains("✓ Looks good"), "the button reads as pressed");
    // Pressing it again clears the looks.
    app.click("diff-looks-good");
    assert_eq!(looked(&app), 0);
    assert!(!tree(&mut app).contains("✓ Looks good"));
}

#[test]
fn a_file_whose_diff_changes_loses_its_tick() {
    let mut app = plain();
    app.click("diff-looks-good");
    let (repo, path) = first_file(&app);
    let changed = reword(&app, &repo, &path);
    app.fakes.daemon.set_diffs(&ledger(), changed);
    app.send(AppAction::Review(R::Retry));
    assert!(!file_looked(&app, &repo, &path), "the reworded file is no longer looked at");
    assert_eq!(looked(&app), file_count(&shown(&app)) - 1, "the others keep theirs");
}

#[test]
fn a_files_circle_toggles_its_look() {
    let mut app = plain();
    let (repo, path) = first_file(&app);
    let id = format!("change-file-look {repo}/{path}");
    app.click(&id);
    assert!(file_looked(&app, &repo, &path));
    assert_eq!(looked(&app), 1);
    app.click(&id);
    assert!(!file_looked(&app, &repo, &path));
}

#[test]
fn the_since_switch_lists_only_the_files_that_changed_after_the_look() {
    let mut app = plain();
    app.click("diff-looks-good");
    let (repo, path) = first_file(&app);
    let changed = reword(&app, &repo, &path);
    let others: Vec<(String, String)> = shown(&app)
        .iter()
        .flat_map(|diff| diff.files.iter().map(move |file| (diff.repo.clone(), file.path.clone())))
        .filter(|(r, p)| (r, p) != (&repo, &path))
        .collect();
    assert!(!others.is_empty());
    app.fakes.daemon.set_diffs(&ledger(), changed);
    app.click("diff-switch-since");
    assert_eq!(
        app.ui.read(|state| state.review.targets_of(&ledger()).preset),
        DiffBase::SinceLastReviewed
    );
    assert!(app.contains("diff-since-caption"));
    assert!(app.contains(&format!("change-file {repo}/{path}")));
    for (other_repo, other_path) in others {
        assert!(
            !app.contains(&format!("change-file {other_repo}/{other_path}")),
            "{other_repo}/{other_path} didn't change"
        );
    }
    app.click("diff-switch-all");
    assert!(!app.contains("diff-since-caption"));
}

#[test]
fn beside_the_terminal_the_switch_waits_for_a_first_look_and_the_looks_bar_is_always_there() {
    let mut app = plain();
    assert!(app.contains("diff-looks-bar"), "the bar is there before any look");
    assert!(!app.contains("diff-switch"), "no look yet, no switch");
    app.click("diff-looks-good");
    assert!(app.contains("diff-switch"));
    app.click("diff-looks-good");
    assert!(!app.contains("diff-switch"), "clearing the looks takes it away again");
}
