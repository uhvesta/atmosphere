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
        spec: RepoDiffSpec { repo: repo.to_owned(), base, options: Default::default() },
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

/// An agent's note on the first file the diff shows, and one draft of the
/// person's on it.
fn note_and_draft_on_first_file(app: &mut App) {
    use humata2_clients::comments::{Anchor, Author, Side};
    let (repo, path) = app.ui.read(|state| {
        let diff = state.workspace.diffs.iter().find(|diff| diff.session == ledger() && !diff.files.is_empty()).expect("a diff");
        (diff.repo.clone(), diff.files[0].path.clone())
    });
    let anchor = Anchor {
        repo,
        path,
        side: Side::New,
        first: 42,
        last: 42,
        quoted: Vec::new(),
        base: None,
        detached: false,
        outdated: false,
    };
    app.fakes.comments.seed_note(&ledger(), anchor.clone(), "Read it through tx.", Author::Agent("claude".into()));
    use humata2_review::comments::CommentsAction as C;
    app.send(AppAction::Comments(C::OpenBox(ledger(), anchor)));
    app.send(AppAction::Comments(C::Typed(ledger(), "Wrap it in one transaction.".into())));
    app.send(AppAction::Comments(C::Close(ledger())));
    app.send(AppAction::Comments(humata2_review::comments::CommentsAction::Open(ledger())));
}

/// UXSIMPLE 3-4: a row counts the drafts and notes on the files it covers,
/// and says nothing when there are none.
#[test]
fn a_layer_row_counts_the_drafts_and_notes_on_its_files() {
    let mut app = stack_app();
    assert!(!app.contains("stack-row-counts-0"), "nothing to count yet");
    note_and_draft_on_first_file(&mut app);
    assert!(app.contains("stack-row-counts-0"), "Whole stack covers the file");
    app.snapshot("stack-layer-counts");
}

/// The file list of the full-screen stack marks a file with a note.
#[test]
fn the_full_screen_file_list_counts_a_files_note_in_orange() {
    let mut app = stack_app();
    note_and_draft_on_first_file(&mut app);
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    let path = app.ui.read(|state| {
        let diff = state.workspace.diffs.iter().find(|diff| diff.session == ledger() && !diff.files.is_empty()).expect("a diff");
        format!("{}/{}", diff.repo, diff.files[0].path)
    });
    assert!(app.contains(&format!("change-file-notes {path}")));
    app.snapshot("stack-layer-counts-full");
}

/// One changed file: `added` lines in, `removed` out, at `tier`, with a few
/// of its lines so the code shows.
fn changed(path: &str, added: u32, removed: u32, tier: humata2_clients::model::FileTier) -> humata2_clients::model::FileDiff {
    use humata2_clients::model::{DiffLine, FileDiff, LineKind};
    let lines = (1..=added.min(5))
        .map(|at| DiffLine {
            kind: LineKind::Added,
            old: None,
            new: Some(at),
            text: format!("line {at} of {path}"),
        })
        .collect();
    FileDiff {
        path: path.to_owned(),
        added,
        removed,
        lines,
        tier,
        ..FileDiff::default()
    }
}

/// `file` with these code lines: each is its kind ('+', '-', ' ' or '@'),
/// old and new numbers, then the text.
fn with_code(
    mut file: humata2_clients::model::FileDiff,
    lines: &[(char, Option<u32>, Option<u32>, &str)],
) -> humata2_clients::model::FileDiff {
    use humata2_clients::model::{DiffLine, LineKind};
    file.lines = lines
        .iter()
        .map(|&(kind, old, new, text)| DiffLine {
            kind: match kind {
                '+' => LineKind::Added,
                '-' => LineKind::Removed,
                '@' => LineKind::Hunk,
                _ => LineKind::Context,
            },
            old,
            new,
            text: text.to_owned(),
        })
        .collect();
    file
}

/// Frame 119's zstd.rs: the codec's first lines.
fn zstd() -> humata2_clients::model::FileDiff {
    let line = |at: u32, text| ('+', None, Some(at), text);
    with_code(
        changed("src/codec/zstd.rs", 212, 0, humata2_clients::model::FileTier::Key),
        &[
            line(1, "use zstd::stream::{encode_all, decode_all};"),
            line(2, ""),
            line(3, "pub struct Zstd { level: i32 }"),
            line(4, ""),
            line(5, "impl Codec for Zstd {"),
        ],
    )
}

/// Frame 120's pages.rs: the hunk that writes through the codec.
fn pages() -> humata2_clients::model::FileDiff {
    with_code(
        changed("src/journal/pages.rs", 148, 52, humata2_clients::model::FileTier::Key),
        &[
            ('@', None, None, "@@ -40,5 +40,6 @@ impl Page"),
            (' ', Some(40), Some(40), "    pub fn write(&mut self, entry: &Entry) -> Result<()> {"),
            ('-', Some(41), None, "        self.buf.extend_from_slice(&entry.bytes);"),
            ('+', None, Some(41), "        let packed = self.codec.encode(&entry.bytes)?;"),
            ('+', None, Some(42), "        self.buf.extend_from_slice(&packed);"),
            (' ', Some(42), Some(43), "        self.len += 1;"),
            (' ', Some(43), Some(44), "        Ok(())"),
            (' ', Some(44), Some(45), "    }"),
        ],
    )
}

/// Frames 119 to 121's stack: api and web; 1 "codec" in both (web moved
/// since it was sent), 2 "journal" in api, 3 "Not committed yet" in api.
fn frame_app() -> App {
    frame_app_listing(true)
}

/// [`frame_app`], with the layers listing their files (`listed`) or not.
fn frame_app_listing(listed: bool) -> App {
    use humata2_clients::model::{FileTier, RepoDiff};
    use humata2_clients::views::LayerDrift;
    let (key, normal, skim) = (FileTier::Key, FileTier::Normal, FileTier::Skim);
    let mut workspace = fixture::design_session(Status::Working);
    for session in &mut workspace.sessions {
        if session.id == ledger() {
            session.repo = "acme/api".to_owned();
        }
    }
    let repo = |name: &str, files| RepoDiff {
        session: ledger(),
        repo: name.to_owned(),
        base: "main".to_owned(),
        files,
        comments: Vec::new(),
    };
    let api = vec![
        zstd(),
        pages(),
        changed("src/journal/writer.rs", 61, 9, normal),
        changed("Cargo.lock", 31, 0, skim),
        changed("src/journal/mod.rs", 31, 0, skim),
        changed("README.md", 3, 0, skim),
    ];
    let web = vec![changed("src/lib/codec.ts", 31, 6, key), changed("package-lock.json", 8, 0, skim)];
    workspace.diffs.retain(|diff| diff.session != ledger());
    workspace.diffs.push(repo("acme/api", api.clone()));
    workspace.diffs.push(repo("acme/web", web.clone()));
    let mut app = App::new(workspace);
    let session = ledger();
    let daemon = &app.fakes.daemon;
    daemon.set_range_files(&session, "acme/api", "a0", "a1", vec![
        zstd(),
        changed("Cargo.lock", 31, 0, skim),
        changed("codec.rs", 14, 0, skim),
        changed("README.md", 3, 0, skim),
    ]);
    daemon.set_range_files(&session, "acme/web", "w0", "w1", web);
    daemon.set_range_files(&session, "acme/api", "a1", "a2", vec![
        pages(),
        changed("src/journal/writer.rs", 61, 9, normal),
        changed("Cargo.lock", 0, 0, skim),
        changed("src/journal/mod.rs", 0, 0, skim),
        changed("README.md", 0, 0, skim),
    ]);
    let range = |base: &str, head: &str| RepoBase::Range { base: base.to_owned(), head: head.to_owned() };
    let main = |repo: &str| RepoDiffSpec {
        repo: repo.to_owned(),
        base: RepoBase::Branch("main".to_owned()),
        options: Default::default(),
    };
    let mut web_part = part("acme/web", range("w0", "w1"), "acme/web#88", 2);
    web_part.drift = Some(LayerDrift::Moved);
    let mut stack = StackView {
        layers: vec![
            layer(
                "codec",
                "Adds the zstd codec to api and web",
                vec![part("acme/api", range("a0", "a1"), "acme/api#412", 4), web_part],
            ),
            layer(
                "journal",
                "Writes journal pages through the codec",
                vec![part("acme/api", range("a1", "a2"), "acme/api#413", 5)],
            ),
            layer(
                "Not committed yet",
                "Work still in the api worktree",
                vec![part(
                    "acme/api",
                    RepoBase::Overlay { base: "a2".to_owned(), paths: vec![".".to_owned()] },
                    "",
                    1,
                )],
            ),
        ],
        whole: vec![main("acme/api"), main("acme/web")],
    };
    if listed {
        let pair = |repo: &str, path: &str| (repo.to_owned(), path.to_owned());
        stack.layers[0].order = vec![pair("acme/api", "src/codec/zstd.rs"), pair("acme/web", "src/lib/codec.ts")];
        stack.layers[1].order =
            vec![pair("acme/api", "src/journal/pages.rs"), pair("acme/api", "src/journal/writer.rs")];
    }
    app.open("ledger", Panel::Journal);
    app.fakes.views.offer_view(
        &ledger(),
        "Review plan: the stack",
        "Three layers",
        "Claude Code",
        ViewContent::ReviewPlan(PlanView { stack: Some(stack), ..PlanView::default() }),
    );
    app.settle();
    app.click("offer-open");
    app.click("view-keep");
    // The frames clip long lines; the person's wrap preference decides.
    if app.ui.read(|state| state.review.wrap) {
        app.send(AppAction::Review(humata2_review::ReviewAction::ToggleWrap));
    }
    app
}

/// Frames 119 to 121 drawn by the app: Whole stack, layer 2 with web left
/// out, and layer 1 in dark with web moved and api's skim files open.
#[test]
fn the_stack_frames_as_the_app_draws_them() {
    let mut app = frame_app();
    app.cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
    assert!(app.contains("stack-caption"));
    assert!(app.contains("change-key acme/api/src/codec/zstd.rs"));
    assert!(!app.contains("stack-drift-3-acme/web"), "Whole stack shows no drift");
    app.snapshot("stack-whole");
    app.click("stack-row-2");
    assert!(app.contains("stack-untouched acme/web"));
    assert!(app.contains("stack-next-layer"));
    app.snapshot("stack-layer");
    app.click("stack-row-3");
    assert!(app.contains("stack-drift-3-acme/web"));
    app.click("stack-skim acme/api");
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.snapshot("stack-skim-drift");
    app.cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
}

#[test]
fn whole_stack_marks_where_each_layer_starts_and_a_click_picks_it() {
    let mut app = frame_app();
    assert!(app.contains("stack-layer-caption acme/api 1"));
    assert!(app.contains("stack-layer-caption acme/api 2"));
    assert!(app.contains("stack-layer-caption acme/web 1"));
    assert!(!app.contains("stack-layer-caption acme/api 3"), "layer 3 lists no files");
    app.click("stack-layer-caption acme/api 2");
    assert!(inside(&mut app, "stack-row-picked", "stack-row-2"));
    assert!(!app.contains("stack-layer-caption acme/api 2"), "a picked layer draws no captions");
}

#[test]
fn a_stack_whose_layers_list_no_files_draws_no_captions() {
    let mut app = frame_app_listing(false);
    assert!(app.contains("stack-list"));
    for id in ["stack-layer-caption acme/api 1", "stack-layer-caption acme/api other"] {
        assert!(!app.contains(id), "{id}");
    }
}
