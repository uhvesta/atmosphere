//! The Diff panel's file list and header in a narrow and an expanded panel
//! (DV-1): the list scrolls, the code keeps room, and the header buttons
//! stay inside the panel.
use super::*;
use humata2_clients::model::FileDiff;
use humata2_review::ReviewAction as R;

/// The "ledger" session's `api` repository holds 200 files, `src/client.rs`
/// among them, and `src/client.rs` is the selected file.
fn many_files_app() -> App {
    let mut workspace = fixture::design();
    let repo = workspace
        .diffs
        .iter_mut()
        .find(|d| d.repo == "api")
        .unwrap();
    let client: FileDiff = repo
        .files
        .iter()
        .find(|f| f.path == "src/client.rs")
        .unwrap()
        .clone();
    repo.files = std::iter::once(client.clone())
        .chain((0..199).map(|n| FileDiff {
            path: format!("src/file_{n:03}.rs"),
            ..client.clone()
        }))
        .collect();
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Pane(PaneAction::Drag(PaneId::Panel, 220.)));
    app.send(AppAction::Review(R::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    app
}

/// In a narrow panel the file list takes at most 220 pt and the code keeps
/// the rest, above the footer.
#[test]
fn a_long_file_list_leaves_room_for_the_code() {
    let mut app = many_files_app();
    // A draft puts the footer's Send button in the footer, the edge to
    // measure the code against.
    app.write_comment("ledger", "api", 43, "Keep these writes in one transaction.");
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let files = window.find("changes-files").bounds();
            let code = window.find("change-code api/src/client.rs").bounds();
            // The Send button sits inside the footer's padding, so its top
            // is just under the footer's top edge.
            let send = window.find("send-comments").bounds();
            assert!(f32::from(files.size.height) <= 220.);
            assert!(f32::from(code.size.height) > 100.);
            assert!(code.origin.y + code.size.height <= send.origin.y + px(0.5));
        })
        .unwrap();
}

/// Expanded, the file list takes the panel's full height and scrolls
/// inside it.
#[test]
fn the_file_list_scrolls_in_the_expanded_panel() {
    let mut app = many_files_app();
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let files = window.find("changes-files").bounds();
            let window_height = window.viewport_size().height;
            assert!(f32::from(files.size.height) > 220.);
            assert!(files.origin.y + files.size.height <= window_height + px(0.5));
        })
        .unwrap();
}

/// A base too long for the chip ellipsizes inside the header, and Expand
/// and Close stay inside the narrow panel.
#[test]
fn a_long_base_keeps_expand_and_close_inside_the_panel() {
    let mut workspace = fixture::design();
    for diff in workspace
        .diffs
        .iter_mut()
        .filter(|d| d.session == SessionId::new("ledger"))
    {
        diff.base = "origin/main...HEAD + staged + unstaged + untracked".into();
    }
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Pane(PaneAction::Drag(PaneId::Panel, 220.)));
    app.send(AppAction::Review(R::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let panel = window.find("side-panel").bounds();
            let right = panel.origin.x + panel.size.width;
            let expand = window.find("expand-panel").bounds();
            let close = window.find("close-panel").bounds();
            assert!(expand.origin.x + expand.size.width <= right + px(0.5));
            assert!(close.origin.x + close.size.width <= right + px(0.5));
        })
        .unwrap();
}
