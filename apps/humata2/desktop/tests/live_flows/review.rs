//! SPEC flows 26.3 (review a 3-repo diff and send comments), 26.14
//! (switch long lines between Wrap and Scroll) and 26.20 (iterate on a
//! document with the agent) on a real daemon, every step included.
use std::path::{Path, PathBuf};

use gpui_kit::Role;
use humata2_app::{AppAction, Route};
use humata2_clients::comments::{Anchor, Side, Status};
use humata2_clients::model::{Agent, DiffBase, SessionId, SessionOptions};
use humata2_review::ReviewAction;
use humata2_review::comments::CommentsAction;
use humata2_session::{DocumentMode, Panel, SessionAction, Tab};

use super::world::World;
use super::{App, agent_has, running_session, runtime};

const REPOS: [&str; 3] = ["api", "ledger", "web"];

/// The session's clone of `name`. Call it once the agent has its first
/// message: the daemon clones before it launches the agent (D441), so one
/// look is enough.
pub(super) fn clone_of(channels: &Path, name: &str) -> PathBuf {
    let children = |folder: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(folder)
            .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default()
    };
    for channel in children(channels) {
        for session in children(&channel.join("sessions")) {
            let clone = session.join(name);
            if clone.join("README.md").exists() {
                return clone;
            }
        }
    }
    panic!(
        "the running session has no clone of {name} under {}",
        channels.display()
    );
}

/// Writes `text` to `path` in `clone`, making its folders.
pub(super) fn write(clone: &Path, path: &str, text: &str) {
    let file = clone.join(path);
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder).expect("make the file's folder");
    }
    std::fs::write(file, text).expect("write the file");
}

/// Lines `1..=count`, each naming its number.
pub(super) fn numbered(count: u32) -> String {
    (1..=count).map(|n| format!("line {n}\n")).collect()
}

/// The repos and paths the app holds for `session`, with files.
fn shown(app: &App, session: &SessionId) -> Vec<(String, Vec<String>)> {
    app.ui.read(|state| {
        let mut shown: Vec<_> = state
            .workspace
            .diffs
            .iter()
            .filter(|diff| &diff.session == session && !diff.files.is_empty())
            .map(|diff| {
                (
                    diff.repo.clone(),
                    diff.files.iter().map(|file| file.path.clone()).collect(),
                )
            })
            .collect();
        shown.sort();
        shown
    })
}

fn send(app: &mut App, action: AppAction) {
    app.cx.update(|cx| app.ui.send(action, cx));
    app.settle();
}

/// Writes a comment on a new-side line and leaves it as a draft.
fn draft(app: &mut App, session: &SessionId, repo: &str, path: &str, line: u32, body: &str) {
    send(
        app,
        AppAction::Review(ReviewAction::SelectFile {
            repo: repo.to_owned(),
            path: path.to_owned(),
        }),
    );
    let anchor = Anchor {
        repo: repo.to_owned(),
        path: path.to_owned(),
        side: Side::New,
        first: line,
        last: line,
        quoted: Vec::new(),
        base: None,
        detached: false,
        outdated: false,
    };
    send(app, AppAction::Comments(CommentsAction::OpenBox(session.clone(), anchor)));
    send(app, AppAction::Comments(CommentsAction::Typed(session.clone(), body.to_owned())));
    send(app, AppAction::Comments(CommentsAction::Close(session.clone())));
}

/// 26.3 steps 1, 2, 4, 6 and 7.
#[test]
fn a_three_repo_diff_is_reviewed_commented_on_and_marked_reviewed() {
    let runtime = runtime();
    let (world, clients, session) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let mac = World::this_mac(&clients)
            .await
            .expect("the workspace has This Mac");
        let repos: Vec<String> = REPOS
            .iter()
            .map(|name| world.repo("owner", name).expect("make a repo"))
            .collect();
        let channel = clients
            .daemon
            .create_channel("payments", repos.clone(), vec![mac.clone()])
            .await
            .expect("create the channel");
        let session = clients
            .daemon
            .create_session(
                &channel,
                Agent::Claude,
                &mac,
                "Ledger schema migration",
                SessionOptions {
                    repos,
                    first_message: "hello agent".to_owned(),
                    ..SessionOptions::default()
                },
            )
            .await
            .expect("create the session");
        agent_has(&world, "hello agent").await;
        (world, clients, session)
    });
    let channels = world
        .conversations()
        .parent()
        .expect("the conversations folder has a parent")
        .join("ch");
    let api = clone_of(&channels, "api");
    let ledger = clone_of(&channels, "ledger");
    let web = clone_of(&channels, "web");
    write(&api, "src/client.rs", &numbered(45));
    write(&ledger, "migrations/0042_accounts.sql", &numbered(12));
    write(&web, "src/cards/Cards.tsx", &numbered(8));

    // 1. The user opens Diff.
    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui
            .read(|state| state.workspace.session(&session).is_some())
    });
    send(&mut app, AppAction::Navigate(Route::Session(session.clone())));
    send(
        &mut app,
        AppAction::Session(SessionAction::SetPanel(Some(Panel::Changes))),
    );

    // 2. Since I last reviewed: nothing was marked yet, so all three repos.
    send(
        &mut app,
        AppAction::Review(ReviewAction::PickBase(DiffBase::SinceLastReviewed)),
    );
    let all = vec![
        ("owner/api".to_owned(), vec!["src/client.rs".to_owned()]),
        (
            "owner/ledger".to_owned(),
            vec!["migrations/0042_accounts.sql".to_owned()],
        ),
        ("owner/web".to_owned(), vec!["src/cards/Cards.tsx".to_owned()]),
    ];
    app.wait_until("three repo groups in the Diff panel", |app| {
        shown(app, &session) == all
    });

    // 3. Full screen to read, then back beside the terminal.
    send(
        &mut app,
        AppAction::ToggleFullScreen(humata2_app::layout::FullScreen::Panel),
    );
    assert_eq!(
        app.ui.read(|state| state.full_screen()),
        Some(humata2_app::layout::FullScreen::Panel)
    );
    send(
        &mut app,
        AppAction::ToggleFullScreen(humata2_app::layout::FullScreen::Panel),
    );

    // 4. A draft on each repo, then a second on client.rs.
    draft(&mut app, &session, "owner/api", "src/client.rs", 40, "Name this error.");
    draft(
        &mut app,
        &session,
        "owner/ledger",
        "migrations/0042_accounts.sql",
        3,
        "Index the account id.",
    );
    draft(&mut app, &session, "owner/web", "src/cards/Cards.tsx", 2, "Show the empty state.");
    draft(&mut app, &session, "owner/api", "src/client.rs", 42, "Retry once here.");

    // 5. client.rs in the editor, Side by side, ↓ to the first change.
    send(
        &mut app,
        AppAction::Session(SessionAction::SelectFile {
            repo: "owner/api".into(),
            path: "src/client.rs".into(),
        }),
    );
    send(&mut app, AppAction::Session(SessionAction::OpenFile));
    send(
        &mut app,
        AppAction::Session(SessionAction::CodeMode {
            repo: "owner/api".into(),
            path: "src/client.rs".into(),
            mode: humata2_session::editor::CodeMode::SideBySide,
        }),
    );
    app.wait_until("the editor to list client.rs's changes", |app| {
        app.view().contains("Next change")
    });
    app.click(Role::Button, "Next change");
    assert_eq!(
        app.ui
            .read(|state| state.session.change(&session, "owner/api", "src/client.rs")),
        Some(0),
        "↓ steps to the first change"
    );

    // 6. Send 4 comments: one message, one tagged line per comment.
    app.click(Role::Button, "Send 4 comments");
    app.wait_until("the agent to get the four comments", |_| {
        let text = world.conversation_text();
        [
            "api/src/client.rs:40",
            "ledger/migrations/0042_accounts.sql:3",
            "web/src/cards/Cards.tsx:2",
            "api/src/client.rs:42",
            "Name this error.",
            "Index the account id.",
            "Show the empty state.",
            "Retry once here.",
        ]
        .iter()
        .all(|part| text.contains(part))
    });
    assert_eq!(
        world.conversation_text().matches("[comment on ").count(),
        4,
        "each comment is typed once"
    );

    // 7. Mark reviewed; a later change is all Since I last reviewed holds.
    send(&mut app, AppAction::Review(ReviewAction::MarkReviewed));
    write(&api, "src/retry.rs", &numbered(3));
    send(
        &mut app,
        AppAction::Review(ReviewAction::PickBase(DiffBase::SinceLastReviewed)),
    );
    app.wait_until("only the change after Mark reviewed", |app| {
        shown(app, &session) == vec![("owner/api".to_owned(), vec!["src/retry.rs".to_owned()])]
    });

    // 8. Later: push api and open its PR from the api group; ledger and web
    // stay unpushed.
    send(
        &mut app,
        AppAction::Review(ReviewAction::Push {
            repo: "owner/api".into(),
        }),
    );
    app.wait_until("api pushed with its pull request", |app| {
        app.ui.read(|state| {
            matches!(
                state
                    .review
                    .pushes
                    .get(&(session.clone(), "owner/api".to_owned())),
                Some(humata2_review::PushState::Pushed {
                    pull_request: Some(7),
                    ..
                })
            )
        })
    });
    assert!(
        world.remote_branches("owner", "api").len() > 1,
        "api's session branch is on its remote"
    );
    assert_eq!(world.remote_branches("owner", "ledger"), vec!["main".to_owned()]);
    assert_eq!(world.remote_branches("owner", "web"), vec!["main".to_owned()]);
    assert_eq!(
        world
            .gh_calls()
            .iter()
            .filter(|call| call.starts_with("pr create"))
            .count(),
        1,
        "one pull request, for api"
    );
    drop(app);
    drop(world);
}

/// Whether the open diff and editor wrap long lines.
fn wraps(app: &App) -> bool {
    app.ui.read(|state| state.review.wrap)
}

/// 26.14 steps 1 and 3–5 (step 2, widening, is the native
/// `widening_a_wrapped_diff_keeps_its_top_line_in_place`).
#[test]
fn long_lines_switch_between_wrap_and_scroll_and_scroll_survives_a_restart() {
    let runtime = runtime();
    let (world, clients, session) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let session = running_session(&world, &clients, "Fix the ledger", "hello agent").await;
        (world, clients, session)
    });
    let channels = world
        .conversations()
        .parent()
        .expect("the conversations folder has a parent")
        .join("ch");
    let long = format!("let total = {};\n", "entry.amount + ".repeat(30));
    write(&clone_of(&channels, "ledger"), "src/client.rs", &long);
    let storage = clients.storage.clone();

    // 1. The diff of a long line wraps.
    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui.read(|state| state.workspace.session(&session).is_some())
    });
    send(&mut app, AppAction::Navigate(Route::Session(session.clone())));
    send(
        &mut app,
        AppAction::Session(SessionAction::SetPanel(Some(Panel::Changes))),
    );
    app.wait_until("the long line in the Diff panel", |app| {
        shown(app, &session) == vec![("owner/ledger".to_owned(), vec!["src/client.rs".to_owned()])]
    });
    assert!(wraps(&app));

    // 3. Settings › Long lines › Scroll switches the open diff at once.
    send(&mut app, AppAction::Navigate(Route::Settings));
    app.click(Role::Tab, "Scroll");
    assert!(!wraps(&app));
    drop(app);

    // 4. After a restart, Scroll is still set.
    let mut again = runtime.block_on(world.clients()).expect("connect again");
    again.storage = storage;
    let mut app = App::open(&runtime, again);
    app.wait_until("the saved settings to load", |app| !wraps(app));

    // 5. Back to Wrap.
    send(&mut app, AppAction::Navigate(Route::Settings));
    app.click(Role::Tab, "Wrap");
    assert!(wraps(&app));
    drop(app);
    drop(world);
}

/// The session's comments as the app holds them.
fn comments_of(app: &App, session: &SessionId) -> Vec<humata2_clients::comments::Comment> {
    app.ui.read(|state| {
        state
            .comments
            .sessions
            .get(session)
            .map(|held| held.comments.clone())
            .unwrap_or_default()
    })
}

/// A new-side anchor on `rfc-brin.md`, lines `first..=last`.
fn rfc_line(first: u32, last: u32) -> Anchor {
    Anchor {
        repo: "owner/ledger".to_owned(),
        path: RFC.to_owned(),
        side: Side::New,
        first,
        last,
        quoted: Vec::new(),
        base: None,
        detached: false,
        outdated: false,
    }
}

const RFC: &str = "rfc-brin.md";

/// The RFC as the agent first writes it: a table on lines 5–8, an ASCII
/// diagram on lines 10–12.
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

/// The rewrite: the sizes on line 7 changed, the diagram (lines 10–12)
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

/// 26.20: comment on a document mid-turn, send two drafts together, then
/// resolve the addressed one and re-attach the one whose lines are gone.
#[test]
fn a_document_is_commented_on_rewritten_and_its_comments_followed() {
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

    // 1–2. The agent writes the RFC; it opens as a tab beside the terminal.
    let ledger = clone_of(&channels, "ledger");
    write(&ledger, RFC, RFC_FIRST);
    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui
            .read(|state| state.workspace.session(&session).is_some())
    });
    send(&mut app, AppAction::Navigate(Route::Session(session.clone())));
    app.wait_until("the RFC among the session's changes", |app| {
        app.ui.read(|state| {
            state.workspace.diffs.iter().any(|diff| {
                diff.session == session
                    && diff.repo == "owner/ledger"
                    && diff.files.iter().any(|file| file.path == RFC)
            })
        })
    });
    send(
        &mut app,
        AppAction::Session(SessionAction::SelectFile {
            repo: "owner/ledger".to_owned(),
            path: RFC.to_owned(),
        }),
    );
    send(&mut app, AppAction::Session(SessionAction::OpenFile));
    let file_tab = app.ui.read(|state| {
        state.session.view().and_then(|view| {
            view.groups
                .first()
                .and_then(|group| group.tabs.iter().position(|tab| matches!(tab, Tab::File { .. })))
        })
    });
    let file_tab = file_tab.expect("the RFC opens as a tab");
    send(
        &mut app,
        AppAction::Session(SessionAction::MoveTab {
            group: 0,
            index: file_tab,
        }),
    );
    assert_eq!(
        app.ui.read(|state| state.session.view().map(|view| view.groups.len())),
        Some(2),
        "the RFC sits in a second group beside the terminal"
    );
    app.wait_until("the RFC's text", |app| {
        app.ui.read(|state| {
            state.session.file_contents.iter().any(|((id, _, path), text)| {
                id == &session && path == RFC && text.as_ref().is_ok_and(|t| t.contains("[range map]"))
            })
        })
    });

    // 3. Side by side; a comment on line 7 is typed at once.
    send(
        &mut app,
        AppAction::Session(SessionAction::DocumentMode {
            repo: "owner/ledger".to_owned(),
            path: RFC.to_owned(),
            mode: DocumentMode::SideBySide,
        }),
    );
    send(
        &mut app,
        AppAction::Comments(CommentsAction::OpenBox(session.clone(), rfc_line(7, 7))),
    );
    send(
        &mut app,
        AppAction::Comments(CommentsAction::Typed(
            session.clone(),
            "Where are the sizes from?".to_owned(),
        )),
    );
    send(&mut app, AppAction::Comments(CommentsAction::Comment(session.clone())));
    app.wait_until("the agent to get the comment on line 7", |_| {
        world
            .conversation_text()
            .contains(&format!("[comment on {RFC}:7]"))
    });
    app.wait_until("the comment to show as sent", |app| {
        comments_of(app, &session)
            .iter()
            .any(|c| c.text == "Where are the sizes from?" && c.status == Status::Sent)
    });

    // 4. Two drafts, then Send 2 comments: one paste with both.
    for (line, text) in [(3, "Name the tables."), (12, "Which way do the arrows point?")] {
        send(
            &mut app,
            AppAction::Comments(CommentsAction::OpenBox(session.clone(), rfc_line(line, line))),
        );
        send(
            &mut app,
            AppAction::Comments(CommentsAction::Typed(session.clone(), text.to_owned())),
        );
        send(&mut app, AppAction::Comments(CommentsAction::Close(session.clone())));
    }
    app.wait_until("two drafts", |app| {
        comments_of(app, &session)
            .iter()
            .filter(|c| c.status == Status::Draft)
            .count()
            == 2
    });
    send(&mut app, AppAction::Comments(CommentsAction::SendAll(session.clone())));
    app.wait_until("the agent to get both drafts", |_| {
        let text = world.conversation_text();
        text.contains(&format!("[comment on {RFC}:3]")) && text.contains(&format!("[comment on {RFC}:12]"))
    });
    assert_eq!(
        world.conversation_text().matches("[comment on ").count(),
        3,
        "each comment is typed once"
    );

    // 5. The agent rewrites the RFC: line 7 changed, the diagram gone.
    write(&ledger, RFC, RFC_SECOND);
    let sizes = |app: &App| {
        comments_of(app, &session)
            .into_iter()
            .find(|c| c.text == "Where are the sizes from?")
    };
    let arrows = |app: &App| {
        comments_of(app, &session)
            .into_iter()
            .find(|c| c.text == "Which way do the arrows point?")
    };
    app.wait_until("the sizes comment to be addressed", |app| {
        sizes(app).is_some_and(|c| c.status == Status::Addressed)
    });
    let addressed = sizes(&app).expect("the sizes comment");
    assert!(
        addressed
            .addressed
            .as_ref()
            .is_some_and(|now| now.iter().any(|line| line.contains("48 KB"))),
        "it shows the lines after the change: {addressed:?}"
    );
    app.wait_until("the arrows comment to offer Re-attach or Drop", |app| {
        arrows(app).is_some_and(|c| c.anchor.detached)
    });
    assert!(
        comments_of(&app, &session)
            .iter()
            .any(|c| c.text == "Name the tables." && c.status == Status::Sent),
        "an untouched passage stays sent"
    );

    // 6. Resolve the addressed one; re-attach the other to the new
    // paragraph and send it again.
    send(
        &mut app,
        AppAction::Comments(CommentsAction::Resolve(session.clone(), addressed.id.clone())),
    );
    app.wait_until("the addressed comment to be resolved", |app| {
        sizes(app).is_none_or(|c| c.status == Status::Resolved)
    });
    let detached = arrows(&app).expect("the arrows comment");
    send(
        &mut app,
        AppAction::Comments(CommentsAction::Reattach(
            session.clone(),
            detached.id.clone(),
            rfc_line(10, 10),
        )),
    );
    app.wait_until("the re-attached comment to be a draft on line 10", |app| {
        arrows(app).is_some_and(|c| !c.anchor.detached && c.anchor.first == 10 && c.status == Status::Draft)
    });
    send(&mut app, AppAction::Comments(CommentsAction::SendAll(session.clone())));
    app.wait_until("the agent to get it on line 10", |_| {
        world
            .conversation_text()
            .contains(&format!("[comment on {RFC}:10]"))
    });
    assert_eq!(world.conversation_text().matches("[comment on ").count(), 4);
}

// ---------------------------------------------------------------------
// Flow 02 steps 10, 11, 13 and 14, by clicks on the real controls.
// ---------------------------------------------------------------------

/// The base the Diff panel's chip names for `session`, as the chip reads it.
fn chip_base(app: &App, session: &SessionId) -> String {
    app.ui.read(|state| {
        state
            .workspace
            .diffs
            .iter()
            .find(|diff| &diff.session == session)
            .map(|diff| diff.base.clone())
            .unwrap_or_default()
    })
}

/// Clicks the base chip and waits for the menu.
fn open_base_menu(app: &mut App, session: &SessionId) {
    // While a read is under way the chip names the picked base (DS-34), so
    // its label is the diff's base only once the read is done.
    app.wait_until("the diff to finish reading", |app| {
        app.ui.read(|state| {
            !state
                .review
                .fetch
                .get(session)
                .is_some_and(|fetch| fetch.loading)
        })
    });
    let chip = format!("Against {}", chip_base(app, session));
    app.click(Role::Button, &chip);
    app.wait_until("the base menu to open", |app| {
        app.view().contains("Compare with")
    });
}

/// Opens the menu, picks the base called `option`, and waits until the
/// panel compares against `expected` with the menu closed.
fn pick_base(app: &mut App, session: &SessionId, option: &str, expected: DiffBase) {
    open_base_menu(app, session);
    app.click(Role::ListBoxOption, option);
    app.wait_until("the picked base to apply and the menu to close", |app| {
        app.ui
            .read(|state| state.review.targets_of(session).preset == expected)
            && !app.view().contains("Compare with")
    });
}

/// The open comment box: first line, last line, text.
fn box_of(app: &App, session: &SessionId) -> Option<(u32, u32, String)> {
    app.ui.read(|state| {
        state
            .comments
            .sessions
            .get(session)
            .and_then(|held| held.editing.as_ref())
            .map(|b| (b.anchor.first, b.anchor.last, b.text.clone()))
    })
}

/// Clicks the box's field and types `text` into it, then waits until the
/// box holds it.
fn type_in_box(app: &mut App, session: &SessionId, text: &str) {
    app.click(Role::MultilineTextInput, "Comment");
    app.type_text(text);
    app.wait_until("the typed comment to reach the box", |app| {
        box_of(app, session).is_some_and(|(_, _, held)| held == text)
    });
}

/// Whether `repo`'s last push left `outcome`: `true` for pushed with a
/// pull request, `false` for failed.
fn push_outcome(app: &App, session: &SessionId, repo: &str, pushed: bool) -> bool {
    app.ui.read(|state| {
        match state.review.pushes.get(&(session.clone(), repo.to_owned())) {
            Some(humata2_review::PushState::Pushed { pull_request, .. }) => {
                pushed && pull_request.is_some()
            }
            Some(humata2_review::PushState::Failed(_)) => !pushed,
            _ => false,
        }
    })
}

/// Flow 02 steps 10, 11 and 13: the base menu, a comment box opened by
/// clicking a diff line, drafts shown and sent together from the panel, and
/// the repo group's Push & PR (failing once, then trying again).
///
/// The session has one repo; the two-repo push is proved by
/// `each_repo_group_pushes_its_own_branch_by_clicking_its_button`.
#[test]
fn the_diff_panel_base_menu_line_comments_and_push_work_by_clicking() {
    let runtime = runtime();
    let (world, clients, session) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let session =
            running_session(&world, &clients, "Ledger schema migration", "hello agent").await;
        (world, clients, session)
    });
    let ledger = clone_of(&world.channels_folder(), "ledger");
    write(&ledger, "src/client.rs", &numbered(6));
    let only_client = vec![("owner/ledger".to_owned(), vec!["src/client.rs".to_owned()])];

    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui
            .read(|state| state.workspace.session(&session).is_some())
    });
    // Opening the session and its Diff panel is not what this test is about
    // (the other live tests drive it); these two go through actions.
    send(&mut app, AppAction::Navigate(Route::Session(session.clone())));
    send(
        &mut app,
        AppAction::Session(SessionAction::SetPanel(Some(Panel::Changes))),
    );
    app.wait_until("the changed file in the Diff panel", |app| {
        shown(app, &session) == only_client
    });

    // 10. The base menu lists the bases (no fork base: this is no fork) and
    // the bookmark moves with Looks good (the header switch does Since I
    // last looked); picking a base compares against it and closes the menu.
    open_base_menu(&mut app, &session);
    let menu = app.view();
    for option in [
        "origin/main (where the branch started)",
        "Uncommitted only",
    ] {
        assert!(menu.contains(option), "the base menu offers {option}");
    }
    assert!(
        !menu.contains("Since the fork from its parent"),
        "a session that is no fork has no fork base"
    );
    app.click(Role::ListBoxOption, "Uncommitted only");
    app.wait_until("Uncommitted only to apply and the menu to close", |app| {
        app.ui.read(|state| {
            state.review.targets_of(&session).preset == DiffBase::UncommittedOnly
        }) && !app.view().contains("Compare with")
    });
    pick_base(
        &mut app,
        &session,
        "origin/main (where the branch started)",
        DiffBase::OriginMain,
    );
    app.wait_until("the changed file against the branch's start", |app| {
        shown(app, &session) == only_client
    });

    // 11. Click a changed line's number: a box opens on it. Type, click a
    // second line: the first is kept as a draft and the box moves.
    app.click(Role::ListBoxOption, "owner/ledger/src/client.rs");
    app.click(Role::Button, "Comment on line 2");
    app.wait_until("the box to open on line 2", |app| {
        box_of(app, &session).is_some_and(|(first, last, text)| (first, last) == (2, 2) && text.is_empty())
    });
    type_in_box(&mut app, &session, "Name this error.");
    app.click(Role::Button, "Comment on line 4");
    app.wait_until("the box to move to line 4", |app| {
        box_of(app, &session).is_some_and(|(first, last, text)| (first, last) == (4, 4) && text.is_empty())
    });
    type_in_box(&mut app, &session, "Retry once here.");
    app.wait_until("both comments to be held as drafts", |app| {
        ["Name this error.", "Retry once here."].iter().all(|text| {
            comments_of(app, &session)
                .iter()
                .any(|c| &c.text == text && c.status == Status::Draft)
        })
    });
    assert!(
        !world.conversation_text().contains("Name this error."),
        "drafts wait; the agent has not got them"
    );
    // The panel offers them together; Send types both into the agent.
    app.wait_until("Send 2 comments in the panel", |app| {
        app.view().contains("Send 2 comments")
    });
    app.click(Role::Button, "Send 2 comments");
    app.wait_until("the agent to get both comments", |_| {
        let text = world.conversation_text();
        // One repo: the paste names files without it (several_clones).
        [
            "[comment on src/client.rs:2 ",
            "[comment on src/client.rs:4 ",
            "Name this error.",
            "Retry once here.",
        ]
        .iter()
        .all(|part| text.contains(part))
    });
    assert_eq!(
        world.conversation_text().matches("[comment on ").count(),
        2,
        "each comment is typed once"
    );
    app.wait_until("both comments to show as sent", |app| {
        let held = comments_of(app, &session);
        ["Name this error.", "Retry once here."]
            .iter()
            .all(|text| held.iter().any(|c| &c.text == text && c.status == Status::Sent))
    });
    assert!(
        !app.view().contains("Send 2 comments"),
        "nothing is left to send"
    );

    // 10 again. Looks good moves the bookmark; the switch's Since I last
    // looked then holds only what changes afterwards.
    send(&mut app, AppAction::Review(ReviewAction::LooksGood));
    write(&ledger, "src/retry.rs", &numbered(3));
    send(&mut app, AppAction::Review(ReviewAction::SinceLooked(true)));
    app.wait_until("only the change after Mark reviewed", |app| {
        shown(app, &session) == vec![("owner/ledger".to_owned(), vec!["src/retry.rs".to_owned()])]
    });

    // 13. Push & PR on the repo's group. With its remote gone the push fails
    // and the group says so; once the remote is back, the same button
    // pushes and names the pull request.
    let remote = world
        .channels_folder()
        .parent()
        .expect("the channels folder has a parent")
        .join("remotes/owner/ledger.git");
    let away = remote.with_extension("away");
    std::fs::rename(&remote, &away).expect("take the remote away");
    open_push_menu(&mut app);
    app.click(Role::Button, "Push & PR: owner/ledger");
    app.wait_until("the push to fail on its group", |app| {
        push_outcome(app, &session, "owner/ledger", false)
    });
    assert_eq!(
        world.remote_branches("owner", "ledger").len(),
        0,
        "nothing reached the missing remote"
    );
    std::fs::rename(&away, &remote).expect("put the remote back");
    open_push_menu(&mut app);
    app.click(Role::Button, "Push & PR: owner/ledger");
    app.wait_until("the group to name its pull request", |app| {
        push_outcome(app, &session, "owner/ledger", true)
    });
    assert!(
        app.view().contains("PR #7"),
        "the group shows the pull request"
    );
    assert!(
        world.remote_branches("owner", "ledger").len() > 1,
        "the session's branch is on the remote"
    );
    assert_eq!(
        world
            .gh_calls()
            .iter()
            .filter(|call| call.starts_with("pr create"))
            .count(),
        1,
        "one pull request, opened by the successful push"
    );
    drop(app);
    drop(world);
}

/// Flow 02 step 14: archiving a session whose work is unpushed asks first
/// (frame 65); Cancel keeps it; "Commit, push and open a PR, then archive"
/// pushes the work and archives; the toast's Undo brings the session back.
#[test]
fn archiving_unpushed_work_asks_first_and_undo_brings_the_session_back() {
    let runtime = runtime();
    let (world, clients, id) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = running_session(&world, &clients, "Fix the ledger", "hello agent").await;
        (world, clients, id)
    });
    // Work that exists only in the session's clone.
    let ledger = clone_of(&world.channels_folder(), "ledger");
    write(&ledger, "src/unsaved.rs", &numbered(3));
    let daemon = clients.daemon.clone();
    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui.read(|state| state.workspace.session(&id).is_some())
    });
    app.show_session(&id);
    app.wait_until("the session page to show", |app| {
        app.ui.read(|state| state.route == Route::Session(id.clone()))
    });

    let active_on_daemon = |runtime: &tokio::runtime::Runtime| {
        runtime
            .block_on(daemon.load())
            .expect("the daemon loads")
            .sessions
            .iter()
            .any(|session| session.id == id && !session.archived)
    };

    // The dialog asks, naming the repo whose work would be lost.
    app.click(Role::Button, "Session actions");
    app.click(Role::Button, "Archive session…");
    app.wait_until("the dialog to ask about unpushed work", |app| {
        app.ui.read(|state| state.session.archiving.is_some())
    });
    let named = app.ui.read(|state| {
        state
            .session
            .archiving
            .as_ref()
            .map(|confirm| {
                confirm
                    .plan
                    .unpushed
                    .iter()
                    .map(|work| work.repo.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });
    assert!(
        named.iter().any(|repo| repo.contains("ledger")),
        "the dialog names the repo with unpushed work: {named:?}"
    );
    assert!(
        app.view().contains("Commit, push and open a PR, then archive"),
        "the dialog offers to push first"
    );

    // Cancel archives nothing.
    app.click(Role::Button, "Cancel");
    app.wait_until("the dialog to close", |app| {
        app.ui.read(|state| state.session.archiving.is_none())
    });
    assert!(
        app.ui
            .read(|state| state.workspace.session(&id).is_some_and(|s| !s.archived)),
        "Cancel leaves the session as it was"
    );
    assert!(active_on_daemon(&runtime), "the daemon still has it active");

    // Push first, then archive: the work reaches the remote, the session
    // leaves, and the toast offers Undo.
    app.click(Role::Button, "Session actions");
    app.click(Role::Button, "Archive session…");
    app.wait_until("the dialog to ask again", |app| {
        app.ui.read(|state| state.session.archiving.is_some())
    });
    app.click(Role::Button, "Commit, push and open a PR, then archive");
    app.wait_until("the session to be archived with its Undo toast", |app| {
        app.ui.read(|state| {
            state
                .workspace
                .session(&id)
                // A live load leaves archived sessions out, so gone counts.
                .is_none_or(|session| session.archived)
                && state.toasts.iter().any(|toast| toast.action.is_some())
        })
    });
    assert!(
        world.remote_branches("owner", "ledger").len() > 1,
        "the unpushed work was pushed before archiving"
    );
    assert!(!active_on_daemon(&runtime), "the daemon no longer lists it as active");

    // Undo on the toast brings it back, on the daemon too.
    app.click(Role::Button, "Undo");
    app.wait_until("the session to be back", |app| {
        app.ui.read(|state| {
            state
                .workspace
                .session(&id)
                .is_some_and(|session| !session.archived)
        })
    });
    assert!(active_on_daemon(&runtime), "the daemon holds it as active again");
    drop(app);
    drop(world);
}

/// Flow 02 step 13 with two repos (D388): each group's Push & PR button is
/// named for its repo, so clicking one pushes that repo's branch only and
/// names its pull request; the other remote stays as it was until its own
/// button is clicked.
#[test]
fn each_repo_group_pushes_its_own_branch_by_clicking_its_button() {
    let runtime = runtime();
    let (world, clients, session) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let mac = World::this_mac(&clients)
            .await
            .expect("the workspace has This Mac");
        let repos: Vec<String> = ["api", "ledger"]
            .iter()
            .map(|name| world.repo("owner", name).expect("make a repo"))
            .collect();
        let channel = clients
            .daemon
            .create_channel("payments", repos.clone(), vec![mac.clone()])
            .await
            .expect("create the channel");
        let session = clients
            .daemon
            .create_session(
                &channel,
                Agent::Claude,
                &mac,
                "Ledger schema migration",
                SessionOptions {
                    repos,
                    first_message: "hello agent".to_owned(),
                    ..SessionOptions::default()
                },
            )
            .await
            .expect("create the session");
        agent_has(&world, "hello agent").await;
        (world, clients, session)
    });
    let channels = world.channels_folder();
    write(&clone_of(&channels, "api"), "src/client.rs", &numbered(4));
    write(&clone_of(&channels, "ledger"), "src/retry.rs", &numbered(3));

    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui
            .read(|state| state.workspace.session(&session).is_some())
    });
    send(&mut app, AppAction::Navigate(Route::Session(session.clone())));
    send(
        &mut app,
        AppAction::Session(SessionAction::SetPanel(Some(Panel::Changes))),
    );
    send(
        &mut app,
        AppAction::Review(ReviewAction::PickBase(DiffBase::SinceLastReviewed)),
    );
    let both = vec![
        ("owner/api".to_owned(), vec!["src/client.rs".to_owned()]),
        ("owner/ledger".to_owned(), vec!["src/retry.rs".to_owned()]),
    ];
    app.wait_until("both repo groups in the Changes panel", |app| {
        shown(app, &session) == both
    });
    assert_eq!(world.remote_branches("owner", "api").len(), 1);
    assert_eq!(world.remote_branches("owner", "ledger").len(), 1);

    // Ledger's button pushes ledger; api's remote is untouched.
    open_push_menu(&mut app);
    app.click(Role::Button, "Push & PR: owner/ledger");
    app.wait_until("ledger's group to name its pull request", |app| {
        push_outcome(app, &session, "owner/ledger", true)
    });
    assert!(
        world.remote_branches("owner", "ledger").len() > 1,
        "ledger's branch is on its remote"
    );
    assert_eq!(
        world.remote_branches("owner", "api").len(),
        1,
        "api's remote was not pushed to"
    );
    assert!(
        app.ui.read(|state| {
            !state
                .review
                .pushes
                .contains_key(&(session.clone(), "owner/api".to_owned()))
        }),
        "api's group has no push outcome yet"
    );

    // Then api's own button.
    open_push_menu(&mut app);
    app.click(Role::Button, "Push & PR: owner/api");
    app.wait_until("api's group to name its pull request", |app| {
        push_outcome(app, &session, "owner/api", true)
    });
    assert!(
        world.remote_branches("owner", "api").len() > 1,
        "api's branch is on its remote"
    );
    drop(app);
    drop(world);
}

/// Opens the Diff footer's Push & PR menu, where each repository's push
/// button lives, unless it is already open.
fn open_push_menu(app: &mut App) {
    if !app.ui.read(|state| state.review.push_menu) {
        send(app, AppAction::Review(ReviewAction::TogglePushMenu));
    }
}
