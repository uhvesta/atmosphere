//! A To-do step's context is added, edited, cleared and cancelled from the
//! drawer's step-context editor (D901).
use super::*;

fn press(app: &mut App, key: &str) {
    app.cx
        .update_window(app.window, |_, window, cx| window.press(key, cx))
        .expect("window stays open");
    app.settle();
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        let key = if c == ' ' { "space".to_owned() } else { c.to_string() };
        press(app, &key);
    }
}

/// Points at step `step`'s row, so its "+ Context" link (shown on hover)
/// can be clicked.
fn hover_step(app: &mut App, step: usize) {
    // The steps sit below the fold of the drawer at the test window's size.
    app.scroll_to("todo-drawer", &format!("todo-step-{step}"));
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.hover(gpui_kit::SharedString::from(format!("todo-step-{step}")), cx);
        })
        .expect("window stays open");
    app.settle();
}

fn add_context(app: &mut App, step: usize) {
    hover_step(app, step);
    app.click(&format!("todo-step-context-add-{step}"));
}

fn step_context(app: &App, key: &str, step: usize) -> Option<String> {
    app.fakes
        .todo
        .entries()
        .iter()
        .find(|e| e.todo.key == key)
        .and_then(|e| e.todo.steps.get(step))
        .and_then(|s| s.markdown_context.clone())
}

fn open_pay3() -> App {
    let mut app = App::new(fixture::design());
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .expect("window stays open");
    app.send(AppAction::Switch(Route::Todo));
    app.click("todo-row PAY-3");
    app
}

#[test]
fn adding_a_step_context_saves_it_and_shows_it_under_the_step() {
    let mut app = open_pay3();
    assert_eq!(step_context(&app, "PAY-3", 0), None);
    add_context(&mut app, 0);
    assert!(app.contains("todo-step-context-edit"));
    type_text(&mut app, "Use the staging DB");
    app.click("todo-step-context-save");
    assert!(!app.contains("todo-step-context-edit"));
    assert_eq!(
        step_context(&app, "PAY-3", 0).as_deref(),
        Some("Use the staging DB")
    );
    assert!(app.contains("todo-step-context-0"));
}

#[test]
fn clearing_a_step_context_and_saving_removes_it() {
    let mut app = open_pay3();
    add_context(&mut app, 0);
    type_text(&mut app, "Use the staging DB");
    app.click("todo-step-context-save");
    assert!(app.contains("todo-step-context-0"));

    app.click("todo-step-context-0");
    assert!(app.contains("todo-step-context-edit"));
    for _ in 0.."Use the staging DB".len() {
        press(&mut app, "backspace");
    }
    app.click("todo-step-context-save");
    assert_eq!(step_context(&app, "PAY-3", 0), None);
    assert!(!app.contains("todo-step-context-0"));
}

#[test]
fn escape_closes_the_step_context_editor_and_keeps_the_drawer_open() {
    let mut app = open_pay3();
    add_context(&mut app, 0);
    assert!(app.contains("todo-step-context-edit"));
    press(&mut app, "escape");
    assert!(!app.contains("todo-step-context-edit"));
    assert!(app.contains("todo-drawer"));
    assert_eq!(step_context(&app, "PAY-3", 0), None);
}
