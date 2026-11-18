//! Input methods, emoji, wide and right-to-left text (micro case 13) in the
//! app against a real `humatad2` and holder running `zsh -f`. Composition
//! goes through the tab's own text input, as the platform's input method
//! reaches it (`Shell::terminal_input`, D484); typed and pasted text goes
//! through the window as the keyboard and ⌘V deliver it.
//!
//! The shell reads back the bytes it got with `xxd -p`, so each check is
//! an answer the shell computed: hex never appears in the typed line.
use gpui_kit::test::{TestInput, TestWindowExt as _};
use gpui_kit::Bounds;
use humata2_app::Route;
use humata2_clients::model::SessionId;
use humata2_flow::Action;

use super::typing::{answered, key, run, shells_of, shows, wait_for, zsh};
use super::App;

/// The tail that has the shell print, in hex, the bytes of what it was given.
const HEX: &str = " | xxd -p";

fn shown_session(app: &App<'_>) -> SessionId {
    app.ui.read(|state| match &state.route {
        Route::Session(id) => id.clone(),
        other => panic!("the app shows a session, not {other:?}"),
    })
}

/// What the input method does to the shown shell tab.
enum Ime<'t> {
    /// Marks `text` as the composition in progress.
    Compose(&'t str),
    /// Commits `text`, replacing the composition.
    Commit(&'t str),
}

fn ime(app: &mut App<'_>, id: &SessionId, step: Ime<'_>) {
    let tab = shells_of(app, id).expect("the session is listed");
    let shell = app.shell.clone();
    app.cx
        .update_window(app.window, |_, window, cx| {
            let bounds = window
                .try_find("terminal")
                .map_or_else(Bounds::default, |terminal| terminal.bounds());
            let handler = shell
                .read(cx)
                .terminal_input(id, tab, bounds, cx)
                .expect("the shell tab has a terminal");
            let mut input = TestInput::new(handler, window);
            match step {
                Ime::Compose(text) => input.compose_text(None, text, None, cx),
                Ime::Commit(text) => input.commit_text(text, cx),
            }
        })
        .expect("the window stays open");
    app.settle();
}

/// Case 13 steps 1, 2 and 3: a composition reaches the shell only when it
/// is committed, once, as UTF-8; an Enter the platform delivers while the
/// composition is open runs nothing. Steps 4, 5 and 7: pasted emoji and
/// typed Persian reach the shell as their exact bytes. Step 6: what the
/// shell prints in wide and right-to-left letters is drawn.
#[test]
fn composed_pasted_and_typed_text_reaches_the_real_shell_as_its_exact_bytes() {
    let runtime = super::runtime();
    let (mut app, world) = zsh(&runtime);
    let id = shown_session(&app);
    run(&mut app, "export LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8");
    answered(&mut app, "xqutf");

    // Steps 1 to 3: compose, have Enter arrive mid-composition, commit.
    app.type_text("printf %s ");
    ime(&mut app, &id, Ime::Compose("にほんご"));
    key(&mut app, "enter");
    ime(&mut app, &id, Ime::Compose("日本語"));
    ime(&mut app, &id, Ime::Commit("日本語"));
    app.type_text(HEX);
    key(&mut app, "enter");
    // Only the committed text, once: no preedit bytes, no early Enter
    // (which would have run `printf %s` alone and printed nothing).
    wait_for(&mut app, &hex("日本語"));
    assert!(
        !shows(&mut app, &hex("にほんご")),
        "the composition's text never reached the shell"
    );
    answered(&mut app, "xqcomposed");

    // Step 4: the family emoji pasted whole.
    app.type_text("printf %s ");
    app.play(&Action::Paste("👨\u{200d}👩\u{200d}👧".to_owned()));
    app.type_text(HEX);
    key(&mut app, "enter");
    wait_for(&mut app, &hex("👨\u{200d}👩\u{200d}👧"));

    // Step 5: a flag pasted whole.
    app.type_text("printf %s ");
    app.play(&Action::Paste("🇯🇵".to_owned()));
    app.type_text(HEX);
    key(&mut app, "enter");
    wait_for(&mut app, &hex("🇯🇵"));

    // Step 7: Persian typed letter by letter.
    app.type_text("printf %s سلام");
    app.type_text(HEX);
    key(&mut app, "enter");
    wait_for(&mut app, &hex("سلام"));

    // Step 6: the shell prints `é 日本語 👨‍👩‍👧 سلام` from octal escapes, so
    // the typed line holds none of those letters; each is drawn.
    let printed = "e\u{301} 日本語 👨\u{200d}👩\u{200d}👧 سلام";
    run(&mut app, &format!("clear; printf '{}\\n'", octal(printed)));
    for letter in ["日", "本", "語", "👨", "س", "م"] {
        wait_for(&mut app, letter);
    }
    answered(&mut app, "xqdrawn");
    drop(app);
    drop(world);
}

fn hex(text: &str) -> String {
    text.bytes().map(|byte| format!("{byte:02x}")).collect()
}

fn octal(text: &str) -> String {
    text.bytes().map(|byte| format!("\\{byte:03o}")).collect()
}
