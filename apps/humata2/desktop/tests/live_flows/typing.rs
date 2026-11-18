//! Typing at a real shell (micro cases 01, 03, 05, 08 and 15) in the app
//! against a real `humatad2` and holder: each key is pressed as the
//! window receives it, goes through the terminal's key encoder, the link
//! and the holder's pty, and the shell's own answer is read back off the
//! drawn grid.
//!
//! The shell is zsh (`zsh -f`, emacs keys), as on this Mac. Every check
//! reads an answer the shell computes, never the echo of the typed line:
//! `| tr a-z A-Z` turns the edited line's words to capitals, which the
//! typed text never holds, so `GIT STATUS --SHORT` on screen means the
//! line that ran was `echo git status --short`, whatever was typed on
//! the way. Words start with `xq` so no app label can match them.
use humata2_app::AppAction;
use humata2_clients::model::{Agent, SessionId};
use humata2_flow::Action;
use humata2_session::SessionAction;

use super::world::{LONG, World};
use super::{App, running_session, runtime};

/// The capitalising tail added to each line before Enter.
pub(super) const UPPER: &str = " | tr a-z A-Z";

pub(super) fn key(app: &mut App<'_>, key: &str) {
    app.play(&Action::Key(key.to_owned()));
}

fn keys(app: &mut App<'_>, name: &str, times: usize) {
    for _ in 0..times {
        key(app, name);
    }
}

pub(super) fn shows(app: &mut App<'_>, what: &str) -> bool {
    app.view().contains(what)
}

/// How many times the shown terminal draws `what`. Read off its rows
/// alone: the view carries the terminal's text twice, as the Log's
/// description and as its text, so counting the whole view doubles.
pub(super) fn count(app: &mut App<'_>, what: &str) -> usize {
    screen(app).join("\n").matches(what).count()
}

/// The rows the shown terminal draws, top to bottom, as its accessible
/// description gives them; empty while no terminal shows.
pub(super) fn screen(app: &mut App<'_>) -> Vec<String> {
    const MARKER: &str = "Log \"Terminal output\" description=\"";
    let view = app.view();
    let Some(start) = view.find(MARKER) else {
        return Vec::new();
    };
    unquote(&view[start + MARKER.len()..])
        .split('\n')
        .map(str::to_owned)
        .collect()
}

/// The text of a `{:?}`-quoted string up to its closing quote, unescaped.
fn unquote(quoted: &str) -> String {
    let mut text = String::new();
    let mut chars = quoted.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') => text.push('\n'),
                Some('r') => text.push('\r'),
                Some('t') => text.push('\t'),
                Some('0') => text.push('\0'),
                Some('u') => {
                    let code: String = chars
                        .by_ref()
                        .skip_while(|c| *c == '{')
                        .take_while(|c| *c != '}')
                        .collect();
                    if let Some(c) = u32::from_str_radix(&code, 16).ok().and_then(char::from_u32) {
                        text.push(c);
                    }
                }
                Some(other) => text.push(other),
                None => break,
            },
            c => text.push(c),
        }
    }
    text
}

/// Waits until the shown terminal has focus and its shell's prompt is
/// drawn: keys typed before then reach no program (the tab refuses input
/// until its screen has arrived) or another element.
pub(super) fn ready(app: &mut App<'_>, prompt: &str) {
    let label = format!("the terminal focused at its {prompt:?} prompt");
    app.wait_within(&label, LONG, |app| at_prompt(app, prompt));
}

/// Whether the shown terminal has focus and draws a row ending in `prompt`.
pub(super) fn at_prompt(app: &mut App<'_>, prompt: &str) -> bool {
    screen(app).iter().any(|row| row.trim_end().ends_with(prompt)) && terminal_focused(app)
}

/// Whether the shown terminal holds the keyboard.
fn terminal_focused(app: &mut App<'_>) -> bool {
    use gpui_kit::test::TestWindowExt as _;
    app.cx
        .update_window(app.window, |_, window, _| {
            window
                .within("terminal-area")
                .try_find("terminal")
                .and_then(|terminal| terminal.focused())
                == Some(true)
        })
        .expect("the window stays open")
}

/// Waits until the grid shows `what`.
pub(super) fn wait_for(app: &mut App<'_>, what: &str) {
    let label = format!("the terminal to show {what:?}");
    app.wait_within(&label, LONG, |app| shows(app, what));
}

/// Waits until the grid no longer shows `what`.
pub(super) fn wait_gone(app: &mut App<'_>, what: &str) {
    let label = format!("the terminal to stop showing {what:?}");
    app.wait_within(&label, LONG, |app| !shows(app, what));
}

/// Types `line`, then Enter.
pub(super) fn run(app: &mut App<'_>, line: &str) {
    app.type_text(line);
    key(app, "enter");
}

/// Runs `echo <word> | tr a-z A-Z` and waits for the capitals: the shell
/// has read every key typed before it.
pub(super) fn answered(app: &mut App<'_>, word: &str) {
    run(app, &format!("echo {word}{UPPER}"));
    wait_for(app, &word.to_ascii_uppercase());
}

pub(super) fn shells_of(app: &App<'_>, id: &SessionId) -> Option<usize> {
    app.ui.read(|state| {
        state
            .workspace
            .sessions
            .iter()
            .find(|session| &session.id == id)
            .map(|session| session.shells.len())
    })
}

/// Opens the app on a running session, adds a shell tab (which takes
/// focus as it opens) and swaps its shell for `zsh -f` with emacs keys.
pub(super) fn zsh<'r>(runtime: &'r tokio::runtime::Runtime) -> (App<'r>, World) {
    let (world, clients, id) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = running_session(&world, &clients, "Fix the ledger", "hello agent").await;
        (world, clients, id)
    });
    let mut app = App::open(runtime, clients);
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .expect("the window stays open");
    app.settle();
    app.show_session(&id);
    app.wait_until("the agent's answer in its terminal", |app| {
        app.view().contains("got: hello agent")
    });
    let before = shells_of(&app, &id).expect("the session is listed");
    app.cx.update(|cx| {
        app.ui.send(
            AppAction::Session(SessionAction::OpenTerminal {
                agent: Agent::Shell,
                split: false,
            }),
            cx,
        )
    });
    app.settle();
    app.wait_until("the session to have one more tab", |app| {
        shells_of(app, &id) == Some(before + 1)
    });
    ready(&mut app, "$");
    run(&mut app, "exec /bin/zsh -f");
    run(&mut app, "bindkey -e");
    answered(&mut app, "xqready");
    (app, world)
}

/// Case 01 steps 2 to 13 and case 03 steps 1, 2 and 6: Backspace, ⌃W,
/// ⌃A, →, ⌃E, ⌃U, ←, ⌃K, ⌃H, ⌃L, ⌥←, ⌥→ and ⌥⌫ edit the line zsh runs.
#[test]
fn editing_keys_fix_the_line_the_real_shell_runs() {
    let runtime = runtime();
    let (mut app, world) = zsh(&runtime);

    // Steps 1 to 9: fix `git stauts --shrot` into `git status --short`.
    app.type_text("echo git stauts --shrot");
    key(&mut app, "backspace"); // --shro
    key(&mut app, "ctrl-w"); // echo git stauts_
    app.type_text("--short");
    key(&mut app, "ctrl-a");
    keys(&mut app, "right", "echo git stau".len());
    key(&mut app, "backspace"); // echo git sta|ts
    key(&mut app, "right"); // echo git stat|s
    app.type_text("u");
    key(&mut app, "ctrl-e");
    app.type_text(UPPER);
    key(&mut app, "enter");
    wait_for(&mut app, "GIT STATUS --SHORT");

    // Step 10: ⌃U clears the whole line.
    app.type_text("echo xqjunk words here");
    key(&mut app, "ctrl-u");
    answered(&mut app, "xqu");
    assert!(!shows(&mut app, "XQJUNK"), "⌃U cleared the line before it ran");

    // Steps 11 and 12: ← then ⌃K drops the tail; ⌃H erases one letter.
    app.type_text("echo xqkh xqtail");
    keys(&mut app, "left", " xqtail".len());
    key(&mut app, "ctrl-k");
    key(&mut app, "ctrl-h");
    app.type_text(UPPER);
    key(&mut app, "enter");
    wait_for(&mut app, "XQK");
    assert!(!shows(&mut app, "XQTAIL"), "⌃K dropped the tail");
    assert!(!shows(&mut app, "XQKH"), "⌃H erased the last letter");

    // Case 03 steps 1, 6 and 2: ⌥← jumps a word back, ⌥⌫ deletes the
    // word before it, ⌥→ jumps to the end of the next word.
    app.type_text("echo xqaa xqbb xqcc");
    key(&mut app, "alt-left"); // before xqcc
    key(&mut app, "alt-backspace"); // echo xqaa xqcc
    key(&mut app, "alt-right"); // after xqcc
    app.type_text("x");
    app.type_text(UPPER);
    key(&mut app, "enter");
    wait_for(&mut app, "XQAA XQCCX");
    assert!(!shows(&mut app, "XQBB"), "⌥⌫ deleted the word before the cursor");

    // Step 13: ⌃L clears the screen; the shell still answers.
    key(&mut app, "ctrl-l");
    wait_gone(&mut app, "GIT STATUS --SHORT");
    answered(&mut app, "xql");
    drop(app);
    drop(world);
}

/// Case 05 steps 4 and 5: ⌃P and ⌃N step through history, and ⌃R finds
/// an earlier line by what it holds; Enter runs what they recalled.
#[test]
fn history_keys_recall_lines_the_real_shell_runs_again() {
    let runtime = runtime();
    let (mut app, world) = zsh(&runtime);
    answered(&mut app, "xqp");
    answered(&mut app, "xqn");
    assert_eq!(count(&mut app, "XQP"), 1);

    // ⌃P ⌃P reaches the xqp line, ⌃N steps forward to xqn, ⌃P back.
    keys(&mut app, "ctrl-p", 2);
    key(&mut app, "ctrl-n");
    key(&mut app, "ctrl-p");
    key(&mut app, "enter");
    app.wait_within("the recalled xqp line to run again", LONG, |app| {
        count(app, "XQP") == 2
    });
    assert_eq!(count(&mut app, "XQN"), 1, "only the recalled line ran");

    // ⌃R then `xqn` finds the xqn line; Enter runs it.
    key(&mut app, "ctrl-r");
    app.type_text("xqn");
    key(&mut app, "enter");
    app.wait_within("the found xqn line to run again", LONG, |app| {
        count(app, "XQN") == 2
    });
    drop(app);
    drop(world);
}

/// Case 08 steps 1, 2, 6, 7, 8 and 9: ⌃C stops a flood, ⌃D ends `cat`
/// on an empty line but not the shell on a typed one, ⌃Z suspends and
/// `fg` resumes, ⌃\ quits.
#[test]
fn control_keys_interrupt_end_and_suspend_real_programs() {
    let runtime = runtime();
    let (mut app, world) = zsh(&runtime);

    // Steps 1 and 2: ⌃C stops `yes`.
    run(&mut app, "yes xqflood");
    app.wait_within("yes to fill the screen", LONG, |app| count(app, "xqflood") >= 3);
    key(&mut app, "ctrl-c");
    // Keys typed before the shell is back at its prompt can go with the
    // interrupted program's input, as in any terminal: wait for it.
    ready(&mut app, "$");
    answered(&mut app, "xqc");

    // Step 6: ⌃D on an empty line ends `cat`.
    run(&mut app, "cat");
    run(&mut app, "xqcat");
    app.wait_within("cat to echo the line", LONG, |app| count(app, "xqcat") >= 2);
    key(&mut app, "ctrl-d");
    answered(&mut app, "xqd");

    // Step 7: ⌃D on a typed line leaves the shell and the line in place.
    app.type_text("echo xqe");
    key(&mut app, "ctrl-d");
    app.type_text(UPPER);
    key(&mut app, "enter");
    wait_for(&mut app, "XQE");

    // Step 8: ⌃Z suspends a program reading the terminal, the shell
    // answers, `fg` brings it back. Not `cat`: macOS's ends with
    // "Interrupted system call" when it is resumed; perl reads on.
    run(&mut app, "perl -pe1");
    key(&mut app, "ctrl-z");
    wait_for(&mut app, "suspended");
    answered(&mut app, "xqz");
    run(&mut app, "fg");
    wait_for(&mut app, "continued");
    run(&mut app, "xqback");
    app.wait_within("the resumed program to echo the line", LONG, |app| {
        count(app, "xqback") >= 2
    });
    assert!(
        !shows(&mut app, "not found"),
        "the line went to the resumed program, not the shell"
    );

    // Step 9: ⌃\ quits it.
    key(&mut app, "ctrl-\\");
    wait_for(&mut app, "quit");
    answered(&mut app, "xqquit");
    drop(app);
    drop(world);
}

/// Case 15 steps 3, 4 and 6 to 11: Esc reaches vim, ⌃Z and `fg` stop and
/// resume it, `:wq` saves and returns to the shell; `less` opens, `/`
/// searches it and `q` leaves it.
#[test]
fn vim_and_less_take_their_keys_from_the_terminal() {
    let runtime = runtime();
    let (mut app, world) = zsh(&runtime);

    run(&mut app, "vim -u NONE -N -n xqv.txt");
    wait_for(&mut app, "[New");
    key(&mut app, "i");
    wait_for(&mut app, "-- INSERT --");
    app.type_text("xqvim");
    key(&mut app, "escape");
    wait_gone(&mut app, "-- INSERT --");

    key(&mut app, "ctrl-z");
    wait_for(&mut app, "suspended");
    answered(&mut app, "xqs");
    run(&mut app, "fg");
    wait_for(&mut app, "xqvim");
    run(&mut app, ":wq");
    run(&mut app, &format!("cat xqv.txt{UPPER}"));
    wait_for(&mut app, "XQVIM");

    run(&mut app, "seq -f 'xqline%g' 1 300 | less");
    wait_for(&mut app, "xqline2");
    assert!(!shows(&mut app, "xqline251"), "less shows the first page");
    app.type_text("/xqline250");
    key(&mut app, "enter");
    wait_for(&mut app, "xqline251");
    app.type_text("q");
    answered(&mut app, "xqq");
    drop(app);
    drop(world);
}
