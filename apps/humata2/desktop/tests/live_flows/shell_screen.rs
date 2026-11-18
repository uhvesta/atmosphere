//! The screen of a real shell in the app against a real `humatad2` and
//! holder (micro cases 02, 06, 07, 10, 11, 14 and 21): pasting, scrolling
//! the history, the find bar, the palette key, and the mouse, selecting
//! or reaching a program that asked for it. Uses the zsh rig of
//! `typing.rs`; answers are read off the drawn grid.
use std::time::Duration;

use gpui_kit::test::{ClickOptions, TestWindowExt as _};
use gpui_kit::{
    ClipboardItem, Modifiers, ModifiersChangedEvent, Pixels, PlatformInput, Point, ScrollDelta, SharedString,
    point, px,
};
use humata2_app::Overlay;
use humata2_flow::Action;

use super::App;
use super::typing::{UPPER, answered, count, key, run, shows, wait_for, wait_gone, zsh};
use super::world::LONG;

const COPY: &str = "cmd-c";
/// The focused group's terminal; other groups and tabs draw their own.
const AREA: &str = "terminal-area";

fn has(app: &mut App<'_>, id: &str) -> bool {
    app.view();
    app.cx
        .update_window(app.window, |_, window, _| window.try_find(SharedString::from(id.to_owned())).is_some())
        .expect("the window stays open")
}

fn click_id(app: &mut App<'_>, id: &str) {
    app.cx
        .update_window(app.window, |_, window, cx| window.click(SharedString::from(id.to_owned()), cx))
        .expect("the window stays open");
    app.settle();
}

fn clipboard(app: &mut App<'_>) -> Option<String> {
    app.cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
}

fn set_clipboard(app: &mut App<'_>, text: &str) {
    app.cx
        .update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned())));
}

/// Where the shown terminal's cells sit in the window. The grid draws its
/// rows itself, with no text element per row, so a character's place
/// comes from its row and column and the size of a cell.
struct Cells {
    origin: Point<Pixels>,
    width: f32,
    height: f32,
}

impl Cells {
    /// Measures the grid of a shell at its prompt. The rows are the drawn
    /// rows; the columns are the shell's own `$COLUMNS`. The grid holds
    /// as many whole cells as fit, so a cell is between a share of the
    /// area with one spare cell and a share with none; half a spare cell
    /// keeps every centre inside its cell.
    fn measure(app: &mut App<'_>) -> Cells {
        run(app, &format!("echo xqcols $COLUMNS{UPPER}"));
        let mut cols = 0;
        app.wait_within("the shell to say its columns", LONG, |app| {
            cols = super::typing::screen(app)
                .iter()
                .find_map(|row| row.trim().strip_prefix("XQCOLS ")?.parse().ok())
                .unwrap_or(0);
            cols > 0
        });
        let rows = super::typing::screen(app).len();
        let bounds = app
            .cx
            .update_window(app.window, |_, window, _| window.within(AREA).find("terminal").bounds())
            .expect("the window stays open");
        Cells {
            origin: bounds.origin,
            width: f32::from(bounds.size.width) / (cols as f32 + 0.5),
            height: f32::from(bounds.size.height) / (rows as f32 + 0.5),
        }
    }

    /// The middle of the `nth` character of the first drawn `needle`, in
    /// window coordinates.
    fn of(&self, app: &mut App<'_>, needle: &str, nth: usize) -> Point<Pixels> {
        let (row, col) = super::typing::screen(app)
            .iter()
            .enumerate()
            .find_map(|(row, text)| {
                let at = text.find(needle)?;
                Some((row, text[..at].chars().count() + nth))
            })
            .unwrap_or_else(|| panic!("{needle:?} is not drawn"));
        point(
            self.origin.x + px(self.width * (col as f32 + 0.5)),
            self.origin.y + px(self.height * (row as f32 + 0.5)),
        )
    }
}

fn wheel(app: &mut App<'_>, lines: f32) {
    app.cx
        .update_window(app.window, |_, window, cx| {
            window
                .within(AREA)
                .scroll("terminal", ScrollDelta::Lines(point(0., lines)), cx)
        })
        .expect("the window stays open");
    app.settle();
}

fn click_cell(app: &mut App<'_>, at: Point<Pixels>, count: usize) {
    app.cx
        .update_window(app.window, |_, window, cx| {
            let origin = window.within(AREA).find("terminal").bounds().origin;
            window.within(AREA).click_with_options(
                "terminal",
                ClickOptions::new().with_offset(at - origin).with_count(count),
                cx,
            )
        })
        .expect("the window stays open");
    app.settle();
}

fn drag(app: &mut App<'_>, from: Point<Pixels>, to: Point<Pixels>, modifiers: Modifiers) {
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.dispatch_event(
                PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                    modifiers,
                    capslock: Default::default(),
                }),
                cx,
            );
            window.drag(from, to, cx);
            window.dispatch_event(
                PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                    modifiers: Modifiers::default(),
                    capslock: Default::default(),
                }),
                cx,
            );
        })
        .expect("the window stays open");
    app.settle();
}

fn palette_open(app: &App<'_>) -> bool {
    app.ui.read(|state| {
        state.palette.open && state.overlays.last() == Some(&Overlay::Palette)
    })
}

/// The `N` of the find bar's `N of total`, if it shows one.
fn find_at(app: &mut App<'_>, total: usize) -> Option<usize> {
    let view = app.view();
    let tail = format!(" of {total}");
    let end = view.find(&tail)?;
    let digits: String = view[..end]
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.chars().rev().collect::<String>().parse().ok()
}

/// Case 06 step 4 and the Cancel branch, case 07 step 1: a paste that
/// hides an end marker stays one paste; lines for a program that did not
/// ask for bracketed paste wait for the user; a 3 MB paste never reaches
/// the shell.
#[test]
fn pastes_reach_a_real_shell_whole_or_not_at_all() {
    let runtime = super::runtime();
    let (mut app, world) = zsh(&runtime);

    // An end marker inside the text cannot end zsh's bracketed paste, so
    // the line after it is not run.
    app.play(&Action::Paste(format!(
        "xqa\x1b[201~echo xqbad{UPPER}\n"
    )));
    wait_for(&mut app, "xqbad");
    key(&mut app, "ctrl-c");
    answered(&mut app, "xqok");
    assert!(!shows(&mut app, "XQBAD"), "the pasted line never ran");

    // `cat` never asks for bracketed paste: three lines wait for the user.
    run(&mut app, "cat");
    app.play(&Action::Paste("xqone\nxqtwo\nxqthree\n".to_owned()));
    app.wait_within("the paste prompt", LONG, |app| has(app, "paste-prompt"));
    assert!(!shows(&mut app, "xqtwo"), "nothing is sent before the answer");
    click_id(&mut app, "paste-cancel");
    assert!(!has(&mut app, "paste-prompt"), "Cancel closes the prompt");
    assert!(!shows(&mut app, "xqtwo"), "Cancel sends nothing");
    app.play(&Action::Paste("xqone\nxqtwo\nxqthree\n".to_owned()));
    app.wait_within("the paste prompt again", LONG, |app| has(app, "paste-prompt"));
    click_id(&mut app, "paste-confirm");
    app.wait_within("cat to echo the pasted lines", LONG, |app| {
        count(app, "xqthree") >= 2
    });
    key(&mut app, "ctrl-d");
    answered(&mut app, "xqcatdone");

    // Case 07 step 1: 3 MB is refused whole; the shell gets none of it.
    let big = "xqbig ".repeat(3 * 1024 * 1024 / 6);
    app.play(&Action::Paste(big));
    answered(&mut app, "xqafter");
    assert!(!shows(&mut app, "xqbig"), "no part of the big paste reached zsh");
    drop(app);
    drop(world);
}

/// Case 10 steps 1, 3, 4, 5, 6, 7 and 8, case 02 step 5, case 21 steps
/// 1 and 2: the wheel scrolls the history and the view holds there while
/// output streams; ⌘Home and ⌘End jump; ⌘F counts matches, ⌘G and ⇧⌘G
/// step through them, a broken expression says so, Esc gives the keyboard
/// back; ⌘K opens the palette, keeps the typing and clears nothing.
#[test]
fn the_history_scrolls_holds_and_finds_over_a_real_shell() {
    let runtime = super::runtime();
    let (mut app, world) = zsh(&runtime);

    run(
        &mut app,
        "seq -f 'xqrow%g.' 1 300; sleep 4; seq -f 'xqlate%g.' 1 50",
    );
    wait_for(&mut app, "xqrow300.");
    wheel(&mut app, 30.);
    wait_gone(&mut app, "xqrow300.");
    assert!(shows(&mut app, "xqrow2"), "the wheel moved into the history");

    // The later output arrives while the view stays where it was put.
    std::thread::sleep(Duration::from_secs(6));
    app.settle();
    assert!(!shows(&mut app, "xqrow300."), "the view held its place");
    assert!(!shows(&mut app, "xqlate"), "new output did not pull the view down");

    // Case 02 step 5: ⌘Home goes to the top, ⌘End back to the bottom.
    key(&mut app, "cmd-home");
    wait_for(&mut app, "xqrow1.");
    key(&mut app, "cmd-end");
    wait_for(&mut app, "xqlate50.");

    // ⌘F: `xqlate4` matches xqlate4. and xqlate40. to xqlate49., 11 rows.
    key(&mut app, "cmd-f");
    app.wait_within("the find bar", LONG, |app| has(app, "find-bar"));
    app.type_text("xqlate4");
    app.wait_within("11 matches counted", LONG, |app| {
        find_at(app, 11).is_some() || shows(app, "11 matches")
    });
    if find_at(&mut app, 11).is_none() {
        key(&mut app, "enter");
    }
    let first = find_at(&mut app, 11).expect("a current match after Enter");
    key(&mut app, "cmd-g");
    let next = find_at(&mut app, 11).expect("⌘G keeps a current match");
    assert_ne!(next, first, "⌘G moves to another match");
    key(&mut app, "shift-cmd-g");
    assert_eq!(find_at(&mut app, 11), Some(first), "⇧⌘G moves back");

    // A broken expression shows its error instead of a count.
    click_id(&mut app, "find-regex");
    key(&mut app, "cmd-a");
    app.type_text("(\\d+");
    assert!(has(&mut app, "find-bar"));
    assert!(
        find_at(&mut app, 11).is_none() && !shows(&mut app, "No matches"),
        "the bar shows the expression's error, not a count"
    );

    // Esc closes the bar; the next keys go to the shell.
    key(&mut app, "escape");
    app.wait_within("the find bar to close", LONG, |app| !has(app, "find-bar"));
    answered(&mut app, "xqesc");

    // ⌘K opens the palette with an empty search; typing goes there, not
    // to the shell; nothing on the terminal is cleared.
    key(&mut app, "cmd-k");
    app.wait_within("the palette, its search empty", LONG, |app| {
        palette_open(app) && app.ui.read(|state| state.palette.query.is_empty())
    });
    app.type_text("mov");
    app.wait_within("the palette search to read mov", LONG, |app| {
        app.ui.read(|state| state.palette.query == "mov")
    });
    assert!(shows(&mut app, "XQESC"), "⌘K cleared nothing");
    key(&mut app, "escape");
    if palette_open(&app) {
        key(&mut app, "escape");
    }
    app.wait_within("the palette to close", LONG, |app| !palette_open(app));
    answered(&mut app, "xqafterpal");
    assert!(
        !shows(&mut app, "movecho"),
        "the palette's typing never reached the shell"
    );
    drop(app);
    drop(world);
}

/// Case 11 steps 1, 2 and 7, case 14 steps 3, 4 and 6: a drag selects
/// and ⌘C copies; a double-click takes a word; ⌘C with nothing selected
/// copies nothing; a program that asked for the mouse gets clicks and the
/// wheel as SGR reports, and a ⇧ drag selects instead; `less` gets the
/// wheel as arrows.
#[test]
fn the_mouse_selects_copies_and_reaches_programs_that_ask() {
    let runtime = super::runtime();
    let (mut app, world) = zsh(&runtime);
    let cells = Cells::measure(&mut app);
    run(&mut app, &format!("echo xqfoo xqword xqbar{UPPER}"));
    wait_for(&mut app, "XQFOO XQWORD XQBAR");

    // ⌘C with nothing selected leaves the clipboard and sends nothing.
    set_clipboard(&mut app, "xqbefore");
    key(&mut app, COPY);
    assert_eq!(clipboard(&mut app).as_deref(), Some("xqbefore"));

    // A double-click takes the word under it.
    let word = cells.of(&mut app, "XQWORD", 2);
    click_cell(&mut app, word, 2);
    key(&mut app, COPY);
    assert_eq!(clipboard(&mut app).as_deref(), Some("XQWORD"));

    // A drag selects from the first word into the last.
    let from = cells.of(&mut app, "XQFOO", 0);
    let to = cells.of(&mut app, "XQBAR", 4);
    drag(&mut app, from, to, Modifiers::default());
    key(&mut app, COPY);
    let copied = clipboard(&mut app).unwrap_or_default();
    assert!(copied.starts_with("XQFOO XQWORD XQBA"), "{copied:?}");
    answered(&mut app, "xqsel");

    // A program asks for SGR mouse reports; `cat -v` shows what it gets
    // once the line ends.
    run(&mut app, "printf '\\e[?1000h\\e[?1006h'; cat -v");
    app.settle();
    let at = cells.of(&mut app, "XQSEL", 1);
    click_cell(&mut app, at, 1);
    key(&mut app, "enter");
    app.wait_within("the click as an SGR report", LONG, |app| {
        shows(app, "[<0;")
    });
    wheel(&mut app, 1.);
    key(&mut app, "enter");
    app.wait_within("the wheel as an SGR report", LONG, |app| {
        shows(app, "[<64;")
    });
    let reports = count(&mut app, "[<0;");
    let from = cells.of(&mut app, "XQSEL", 0);
    let to = cells.of(&mut app, "XQSEL", 3);
    drag(&mut app, from, to, Modifiers::shift());
    key(&mut app, "enter");
    app.settle();
    assert_eq!(count(&mut app, "[<0;"), reports, "a ⇧ drag sends no report");
    set_clipboard(&mut app, "xqbefore");
    key(&mut app, COPY);
    let copied = clipboard(&mut app).unwrap_or_default();
    assert!(copied.starts_with("XQS"), "a ⇧ drag selects: {copied:?}");
    key(&mut app, "ctrl-c");
    run(&mut app, "printf '\\e[?1000l\\e[?1006l'");
    answered(&mut app, "xqmouse");

    // `less` asks for no mouse: on its screen the wheel moves it down.
    run(&mut app, "seq -f 'xq%g.' 1 300 | less");
    wait_for(&mut app, "xq1.");
    wheel(&mut app, -10.);
    wait_gone(&mut app, "xq1.");
    app.type_text("q");
    answered(&mut app, "xqless");
    drop(app);
    drop(world);
}

fn toasts(app: &App<'_>) -> usize {
    app.ui.read(|state| state.toasts.len())
}

/// Dispatches an Edit menu item to the focused element, as the menu does.
fn edit_menu(app: &mut App<'_>, action: Box<dyn gpui_kit::Action>) {
    app.cx
        .update_window(app.window, |_, window, cx| window.dispatch_action(action, cx))
        .expect("the window stays open");
    app.settle();
}

/// Case 02 step 4, case 11 step 9 and case 06 step 5: ⌘A then ⌘C, and
/// Edit > Select All then Edit > Copy, copy the whole history of a real
/// shell; Edit > Paste types the clipboard into it.
#[test]
fn select_all_and_the_edit_menu_reach_a_real_shell() {
    use gpui_kit::component::input;
    let runtime = super::runtime();
    let (mut app, world) = zsh(&runtime);
    let cells = Cells::measure(&mut app);
    run(&mut app, "seq -f 'xqh%g.' 1 200");
    wait_for(&mut app, "xqh200.");

    set_clipboard(&mut app, "xqbefore");
    key(&mut app, "cmd-a");
    key(&mut app, COPY);
    let copied = clipboard(&mut app).unwrap_or_default();
    assert!(copied.contains("xqh1.\n") && copied.contains("xqh200."), "⌘A takes the whole history");

    set_clipboard(&mut app, "xqbefore");
    edit_menu(&mut app, Box::new(input::SelectAll));
    edit_menu(&mut app, Box::new(input::Copy));
    let copied = clipboard(&mut app).unwrap_or_default();
    assert!(copied.contains("xqh1.\n") && copied.contains("xqh200."), "Edit > Copy takes the selection");

    // Clicking clears the selection before the paste.
    let at = cells.of(&mut app, "xqh200.", 0);
    click_cell(&mut app, at, 1);
    set_clipboard(&mut app, "echo xqmenu");
    edit_menu(&mut app, Box::new(input::Paste));
    app.type_text(UPPER);
    key(&mut app, "enter");
    wait_for(&mut app, "XQMENU");
    drop(app);
    drop(world);
}

/// Case 19 step 1 and case 06's exited-shell branch: once the shell
/// exits, the tab says so instead of claiming the link is down, and what
/// is typed or pasted is not sent but told.
#[test]
fn an_exited_shell_says_so_and_tells_what_was_not_sent() {
    let runtime = super::runtime();
    let (mut app, world) = zsh(&runtime);
    run(&mut app, "exit");
    app.wait_within("the tab to say its shell exited", LONG, |app| has(app, "terminal-exited"));

    let before = toasts(&app);
    run(&mut app, "ls");
    app.wait_within("a toast about the typing", LONG, |app| toasts(app) > before);

    app.play(&Action::Paste("echo xqlate".to_owned()));
    app.settle();
    assert!(has(&mut app, "terminal-exited"), "the tab still says the shell exited");
    assert!(!has(&mut app, "disconnected-banner"), "an exited shell is not a lost link");
    assert!(!shows(&mut app, "xqlate"), "the paste is not typed");
    drop(app);
    drop(world);
}

/// Case 14 step 5 and case 10's ⌘PgUp branch: with ⇧ held the wheel
/// scrolls the history of a shell whose program took the mouse and sends
/// it no report; in `less`, on the alternate screen, ⌘PgUp is its PageUp.
#[test]
fn shift_wheel_keeps_the_history_and_page_up_reaches_less() {
    let runtime = super::runtime();
    let (mut app, world) = zsh(&runtime);
    let cells = Cells::measure(&mut app);
    run(&mut app, "seq -f 'xqs%g.' 1 200");
    wait_for(&mut app, "xqs200.");
    run(&mut app, "printf '\\e[?1000h\\e[?1006h'; cat -v");
    app.settle();

    let at = cells.of(&mut app, "xqs200.", 1);
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.dispatch_event(
                gpui_kit::InputEvent::to_platform_input(gpui_kit::ScrollEvent {
                    position: at,
                    delta: ScrollDelta::Lines(point(0., 400.)),
                    modifiers: Modifiers::shift(),
                    ..Default::default()
                }),
                cx,
            );
            // ⇧ is let go, as a hand does; the window keeps the last
            // modifiers it saw, so the next wheel would be ⇧'s too.
            window.dispatch_event(
                PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                    modifiers: Modifiers::default(),
                    capslock: Default::default(),
                }),
                cx,
            );
        })
        .expect("the window stays open");
    app.settle();
    wait_for(&mut app, "xqs1.");

    // A key brings the view back to the bottom: `cat -v` got no wheel
    // report (`[<64;` up to `[<69;`, ⇧ included), though a plain notch
    // still reaches it.
    key(&mut app, "enter");
    wait_gone(&mut app, "xqs1.");
    assert_eq!(count(&mut app, "[<6"), 0, "the ⇧ wheel sends no report");
    wheel(&mut app, 1.);
    key(&mut app, "enter");
    app.wait_within("the plain wheel as an SGR report", LONG, |app| shows(app, "[<64;"));
    key(&mut app, "ctrl-c");
    run(&mut app, "printf '\\e[?1000l\\e[?1006l'");
    answered(&mut app, "xqwheel");

    run(&mut app, "seq -f 'xqp%g.' 1 300 | less");
    wait_for(&mut app, "xqp1.");
    app.type_text(" ");
    wait_gone(&mut app, "xqp1.");
    key(&mut app, "cmd-pageup");
    wait_for(&mut app, "xqp1.");
    app.type_text("q");
    answered(&mut app, "xqpage");
    drop(app);
    drop(world);
}
