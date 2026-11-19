//! Flow 04 steps 3 to 8 on a real `humatad2`, with the clicks a person
//! makes on a document page: the mode switch and Link scrolling, a click on
//! a source line to open the comment box, a shift-click for a range, typing
//! and Comment, drafts left behind and sent together with Send N comments,
//! then Resolve and Re-attach after the agent rewrites the file. The agent
//! is the rig's: the test writes the rewrite itself, as `review.rs` does.
//!
//! `review.rs::a_document_is_commented_on_rewritten_and_its_comments_followed`
//! covers the same comments by app actions; this file clicks the controls.
//!
//! The session is a channel session, so the document is in a repo clone. A
//! Desk session keeps it in the session's own folder, and no file root names
//! that folder yet (flows/04 notes the milestone).
use std::path::Path;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Modifiers, Role};
use humata2_app::AppAction;
use humata2_flow::{Action, Target};
use humata2_clients::comments::{Comment, Status};
use humata2_clients::model::SessionId;
use humata2_session::{DocumentMode, SessionAction, Tab};

use super::review::clone_of;
use super::world::World;
use super::{App, running_session, runtime};

const RFC: &str = "rfc-brin.md";

/// The RFC as the agent first writes it: a table on lines 5 to 8, an ASCII
/// diagram on lines 10 to 12.
const RFC_FIRST: &str = "# BRIN rollout\n\
\n\
Roll BRIN indexes out to the ledger tables.\n\
\n\
| table   | rows  | size  |\n\
|---------|-------|-------|\n\
| entries | 4.1 B | 64 KB |\n\
| posts   | 2.0 B | 32 KB |\n\
\n\
  [entries] --brin--> [range map]\n\
       |\n\
  [posts]   --brin--> [range map]\n";

/// The rewrite: the sizes on line 7 changed, the diagram (lines 10 to 12)
/// gone, so line 12 is past the end; a new paragraph on line 10.
const RFC_SECOND: &str = "# BRIN rollout\n\
\n\
Roll BRIN indexes out to the ledger tables.\n\
\n\
| table   | rows  | size  |\n\
|---------|-------|-------|\n\
| entries | 4.1 B | 48 KB (pg_relation_size, prod, 2026-11-14) |\n\
| posts   | 2.0 B | 32 KB |\n\
\n\
The range maps are built one table at a time, entries first.\n";

const SIZES: &str = "Where are the sizes from?";
const TABLES: &str = "Name the tables.";
const ARROWS: &str = "Which way do the arrows point?";

fn write(folder: &Path, text: &str) {
    std::fs::write(folder.join(RFC), text).expect("write the document");
}

/// The session's comments as the app holds them.
fn comments_of(app: &App, session: &SessionId) -> Vec<Comment> {
    app.ui.read(|state| {
        state
            .comments
            .sessions
            .get(session)
            .map(|held| held.comments.clone())
            .unwrap_or_default()
    })
}

fn comment_with(app: &App, session: &SessionId, text: &str) -> Option<Comment> {
    comments_of(app, session).into_iter().find(|c| c.text == text)
}

/// The open box's lines and text.
fn boxed(app: &App, session: &SessionId) -> Option<(u32, u32, String)> {
    app.ui.read(|state| {
        state
            .comments
            .sessions
            .get(session)
            .and_then(|held| held.editing.as_ref())
            .map(|b| (b.anchor.first, b.anchor.last, b.text.clone()))
    })
}

/// The document's mode and whether its scrolling is linked.
fn shown(app: &App, repo: &str) -> (DocumentMode, bool) {
    app.ui.read(|state| {
        let view = state
            .session
            .view()
            .map(|view| view.document(repo, RFC))
            .unwrap_or_default();
        (view.mode, view.linked)
    })
}

/// A shift-click on a source line. The driver's click has no modifiers, so
/// this presses the line's gutter with Shift held, as a person does.
fn shift_click(app: &mut App, line_id: &'static str) {
    app.settle();
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.click_with_modifiers(line_id, Modifiers::shift(), cx)
        })
        .expect("the line is drawn");
    app.settle();
}

/// Clicks the comment box's field and types `text` in it.
fn write_in_the_box(app: &mut App, session: &SessionId, text: &str) {
    app.click(Role::MultilineTextInput, "Comment");
    app.type_text(text);
    app.wait_until("the typed comment to reach the box", |app| {
        boxed(app, session).is_some_and(|(_, _, held)| held == text)
    });
}

/// The script: flow 04 steps 3 to 8. `folder` is where the document lives
/// on disk; `repo` is how the app names it ("" for a Desk session's own
/// folder).
fn drive(app: &mut App, world: &World, session: &SessionId, repo: &str, folder: &Path) {
    // 3. The document opens as a tab and moves into a group beside the
    // terminal by dragging its tab onto the right edge. It shows as a page
    // in Preview.
    app.cx.update(|cx| {
        app.ui.send(
            AppAction::Session(SessionAction::SelectFile {
                repo: repo.to_owned(),
                path: RFC.to_owned(),
            }),
            cx,
        );
        app.ui.send(AppAction::Session(SessionAction::OpenFile), cx);
    });
    app.settle();
    let opened = app
        .ui
        .read(|state| {
            state.session.view().and_then(|view| {
                view.groups
                    .first()
                    .and_then(|group| group.tabs.iter().position(|tab| matches!(tab, Tab::File { .. })))
            })
        });
    assert!(opened.is_some(), "the document opens as a tab");
    app.play(&Action::Drag {
        from: Target::new(Role::Tab, RFC),
        // The zone is drawn only while a tab is dragged; the driver looks
        // for it once the drag has begun.
        to: Target::new(Role::Region, "Drop the tab beside the terminal"),
    });
    assert_eq!(
        app.ui.read(|state| state.session.view().map(|view| view.groups.len())),
        Some(2),
        "the document sits in a second group beside the terminal"
    );
    app.wait_until("the document's text", |app| {
        app.ui.read(|state| {
            state.session.file_contents.iter().any(|((id, r, path), text)| {
                id == session
                    && r == repo
                    && path == RFC
                    && text.as_ref().is_ok_and(|t| t.contains("[range map]"))
            })
        })
    });
    assert_eq!(shown(app, repo).0, DocumentMode::Preview, "it opens as a page");

    // 4. The mode switch, then Link scrolling in Side by side.
    app.click(Role::RadioButton, "Source");
    assert_eq!(shown(app, repo).0, DocumentMode::Source);
    app.click(Role::RadioButton, "Side by side");
    assert_eq!(shown(app, repo).0, DocumentMode::SideBySide);
    assert!(shown(app, repo).1, "scrolling starts linked");
    // Scrolling the page by hand unticks Link scrolling; the box ticks it
    // again, and clicking the box unticks and ticks it as well.
    app.play(&Action::Scroll {
        target: Target::new(Role::Document, format!("{RFC} preview")),
        x: 0,
        y: -120,
    });
    assert!(!shown(app, repo).1, "scrolling the page by hand unticks it");
    app.click(Role::CheckBox, "Link scrolling");
    assert!(shown(app, repo).1, "ticked again");
    app.click(Role::CheckBox, "Link scrolling");
    assert!(!shown(app, repo).1, "unticked");
    app.click(Role::CheckBox, "Link scrolling");
    assert!(shown(app, repo).1, "ticked again");

    // 5. Click line 7, type, Comment: typed into the terminal at once.
    app.click(Role::Button, "Comment on line 7");
    app.wait_until("the box to open on line 7", |app| {
        boxed(app, session).is_some_and(|(first, last, text)| (first, last) == (7, 7) && text.is_empty())
    });
    write_in_the_box(app, session, SIZES);
    app.click(Role::Button, "Comment");
    app.wait_until("the agent to get the comment on line 7", |_| {
        world.conversation_text().contains(&format!("[comment on {RFC}:7]"))
    });
    app.wait_until("the comment to show as sent", |app| {
        comment_with(app, session, SIZES).is_some_and(|c| c.status == Status::Sent)
    });
    assert!(boxed(app, session).is_none(), "the box closes once sent");

    // 6. A range by shift-click, then a second comment, both left as drafts.
    app.click(Role::Button, "Comment on line 3");
    app.wait_until("the box to open on line 3", |app| {
        boxed(app, session).is_some_and(|(first, last, _)| (first, last) == (3, 3))
    });
    shift_click(app, "doc-line-4");
    assert_eq!(
        boxed(app, session).map(|(first, last, _)| (first, last)),
        Some((3, 4)),
        "the shift-click stretched the box to a range"
    );
    write_in_the_box(app, session, TABLES);
    app.click(Role::Button, "Comment on line 12");
    app.wait_until("the box to move to line 12", |app| {
        boxed(app, session).is_some_and(|(first, last, text)| (first, last) == (12, 12) && text.is_empty())
    });
    write_in_the_box(app, session, ARROWS);
    app.wait_until("both drafts to be held", |app| {
        [TABLES, ARROWS].iter().all(|text| {
            comment_with(app, session, text).is_some_and(|c| c.status == Status::Draft)
        })
    });
    assert_eq!(
        comment_with(app, session, TABLES).map(|c| (c.anchor.first, c.anchor.last)),
        Some((3, 4)),
        "the first draft keeps its range"
    );
    assert!(
        !world.conversation_text().contains(TABLES) && !world.conversation_text().contains(ARROWS),
        "drafts wait; the agent has not got them"
    );
    app.wait_until("Send 2 comments in the link bar", |app| {
        app.view().contains("Send 2 comments")
    });
    app.click(Role::Button, "Send 2 comments");
    app.wait_until("the agent to get both drafts", |_| {
        let text = world.conversation_text();
        text.contains(TABLES) && text.contains(ARROWS)
    });
    app.wait_until("both to show as sent", |app| {
        [TABLES, ARROWS].iter().all(|text| {
            comment_with(app, session, text).is_some_and(|c| c.status == Status::Sent)
        })
    });

    // 7. The agent rewrites the document (the test writes it): the sizes
    // comment is addressed with the lines after, the diagram's comment
    // loses its lines.
    write(folder, RFC_SECOND);
    app.wait_until("the sizes comment to be addressed", |app| {
        comment_with(app, session, SIZES).is_some_and(|c| c.status == Status::Addressed)
    });
    let addressed = comment_with(app, session, SIZES).expect("the sizes comment");
    assert!(
        addressed
            .addressed
            .as_ref()
            .is_some_and(|now| now.iter().any(|line| line.contains("48 KB"))),
        "it shows the lines after the change: {addressed:?}"
    );
    app.wait_until("the arrows comment to lose its lines", |app| {
        comment_with(app, session, ARROWS).is_some_and(|c| c.anchor.detached)
    });

    // 8. The addressed one already reads done, with Reopen and no Resolve
    // (D851); Re-attach the other to the new paragraph and send it again.
    app.wait_until("the addressed comment to read done", |app| {
        app.view().contains("Done by ")
    });
    assert!(
        comment_with(app, session, SIZES).is_some_and(|c| c.status == Status::Addressed),
        "a done comment stays listed as addressed until someone reopens it"
    );
    app.wait_until("Re-attach on the detached comment", |app| {
        app.view().contains("Re-attach")
    });
    app.click(Role::Button, "Re-attach");
    app.click(Role::Button, "Comment on line 10");
    app.wait_until("the comment to be a draft on line 10", |app| {
        comment_with(app, session, ARROWS)
            .is_some_and(|c| !c.anchor.detached && c.anchor.first == 10 && c.status == Status::Draft)
    });
    app.wait_until("Send 1 comment in the link bar", |app| {
        app.view().contains("Send 1 comment")
    });
    app.click(Role::Button, "Send 1 comment");
    app.wait_until("the agent to get it on line 10", |_| {
        world.conversation_text().contains(&format!("[comment on {RFC}:10]"))
    });
    app.wait_until("the re-attached comment to be sent", |app| {
        comment_with(app, session, ARROWS).is_some_and(|c| c.status == Status::Sent)
    });
}

/// Flow 04 steps 3 to 8, clicked, on a channel session.
#[test]
fn a_documents_page_is_clicked_through_comments_drafts_rewrite_resolve_and_reattach() {
    let runtime = runtime();
    let (world, clients, session) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let session =
            running_session(&world, &clients, "BRIN rollout RFC", "write rfc-brin.md").await;
        (world, clients, session)
    });
    let channels = world
        .conversations()
        .parent()
        .expect("the conversations folder has a parent")
        .join("ch");
    let ledger = clone_of(&channels, "ledger");
    write(&ledger, RFC_FIRST);
    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui.read(|state| state.workspace.session(&session).is_some())
    });
    app.show_session(&session);
    app.wait_until("the RFC among the session's changes", |app| {
        app.ui.read(|state| {
            state.workspace.diffs.iter().any(|diff| {
                diff.session == session
                    && diff.repo == "owner/ledger"
                    && diff.files.iter().any(|file| file.path == RFC)
            })
        })
    });
    drive(&mut app, &world, &session, "owner/ledger", &ledger);
    drop(app);
    drop(world);
}
