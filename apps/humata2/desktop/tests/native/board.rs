//! Boards (frames 21-24).
use super::*;

#[test]
fn board_reference_images() {
    use humata2_board::{BoardAction as B, Mode};
    use humata2_clients::model::{ItemId, Scope};
    let mut app = App::new(fixture::design());
    app.fold_quiet_channels();
    app.send(AppAction::Switch(Route::Board(Scope::Global)));
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.snapshot("21-board-kanban");
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.click("board-card PAY-3");
    app.snapshot("22-board-item-open");
    app.click("board-close");
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::Board(B::Mode(Mode::Table)));
    app.snapshot("23-board-table");
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.send(AppAction::Board(B::Mode(Mode::Dashboard)));
    app.snapshot("24-board-dashboard");
    app.click("board-tile-progress");
    assert!(app.contains("board-table INF-2"));
    assert!(!app.contains("board-table PAY-12"));
    app.click("board-clear-filter");
    app.click("board-table PAY-3");
    app.click("board-status");
    app.click("board-move-In review");
    assert_eq!(
        app.ui.read(|s| s
            .workspace
            .items
            .iter()
            .find(|i| i.id == ItemId::new("PAY-3"))
            .unwrap()
            .status),
        humata2_clients::model::ItemStatus::InReview
    );
}

#[test]
fn board_creates_in_selected_column_and_answers_the_item_request() {
    use humata2_board::BoardAction as B;
    use humata2_clients::model::{ItemId, ItemStatus as S, Scope};
    let mut app = App::new(fixture::design());
    app.send(AppAction::Switch(Route::Board(Scope::Global)));
    app.click("board-add-Backlog");
    app.send(AppAction::Board(B::NewTitle("A new milestone".into())));
    app.click("board-create-item");
    assert!(app.ui.read(|s| {
        s.workspace.items.iter().any(|i| {
            i.title == "A new milestone" && i.status == S::Backlog && i.scope == Scope::Global
        })
    }));
    app.click("board-card PAY-3");
    app.click("board-approve");
    assert_eq!(
        app.ui.read(|s| s
            .workspace
            .items
            .iter()
            .find(|i| i.id == ItemId::new("PAY-3"))
            .unwrap()
            .status),
        S::Done
    );
    assert!(!app.contains("board-approve"));
    app.send(AppAction::Switch(Route::Channel(
        humata2_clients::model::ChannelId::new("infra"),
    )));
    app.click("channel-tab-Board");
    assert!(app.contains("board-card INF-2"));
    assert!(!app.contains("board-card PAY-3"));
}
