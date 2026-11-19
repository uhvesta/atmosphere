use super::*;
use humata2_clients::model::{DiffBase, RepoBase};
use humata2_review::order::{OrderFile, bottom_up};
use humata2_review::{PushState, RepoPicker};
use std::cell::RefCell;
use std::collections::HashMap;

/// The footer's count of drafts for GitHub (DS-55): "4 drafts · 3 by
/// Copilot, 1 by you", naming the session's agent.
pub(super) fn drafts_line(state: &AppState, session: &SessionId, agent: usize, you: usize) -> String {
    let total = agent + you;
    if total == 0 {
        return "No drafts".to_owned();
    }
    let name = state
        .workspace
        .sessions
        .iter()
        .find(|each| &each.id == session)
        .map_or("the agent", |each| match each.agent {
            humata2_clients::model::Agent::Claude => "Claude Code",
            other => other.name(),
        });
    let mut by = Vec::new();
    if agent > 0 {
        by.push(format!("{agent} by {name}"));
    }
    if you > 0 {
        by.push(format!("{you} by you"));
    }
    format!("{total} draft{} · {}", if total == 1 { "" } else { "s" }, by.join(", "))
}

/// The drafts for GitHub of `session`, by the agent and by the person.
pub(super) fn github_draft_counts(state: &AppState, session: &SessionId) -> (usize, usize) {
    let drafts = state.comments.github_drafts(session);
    let by_you = drafts
        .iter()
        .filter(|draft| matches!(draft.author, humata2_clients::comments::Author::User))
        .count();
    (drafts.len() - by_you, by_you)
}

/// The pull requests `session` reviews, as the workspace lists them.
fn review_pulls<'a>(state: &'a AppState, session: &Session) -> Vec<&'a humata2_clients::model::PullRequest> {
    state
        .workspace
        .pulls
        .iter()
        .filter(|pull| pull.session.as_ref() == Some(&session.id))
        .collect()
}

/// The review's first line (PR-6): its pull request, what it is compared
/// with and how many repositories it spans (frame 68).
pub(super) fn review_line(state: &AppState, look: Look, session: &Session, base: &str, repos: usize) -> Div {
    let pulls = review_pulls(state, session);
    let repos = if pulls.is_empty() { repos } else { pulls.len() };
    let name = pulls
        .first()
        .map_or_else(|| "Pull request".to_owned(), |pull| format!("PR #{}", pull.number));
    // The line ellipsizes before the header's buttons give way.
    div().min_w_0().overflow_hidden().child(row()
        .id("review-pr-line")
        .test_support()
        .gap(px(6.))
        .min_w_0()
        .overflow_hidden()
        .child(div().flex_shrink_0().child(icon(IconName::GitPullRequest, look.secondary, 12.)))
        .child({
            let line = format!(
                "{name} against {base} · {repos} repo{}",
                if repos == 1 { "" } else { "s" }
            );
            full_text(
                secondary(look, line.clone())
                    .id("review-pr-line-text")
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis(),
                line,
            )
        }))
}

thread_local! {
    /// The focus the Diff panel last scrolled to: the review and file, and
    /// the focusing change's number.
    static REVEALED: RefCell<Option<(String, u64)>> = const { RefCell::new(None) };
}

/// Scrolls the review's list so the focused row is inside its view, once per
/// focus. Waits for the row's bounds and the view's.
fn reveal_focus(
    handle: &ScrollHandle,
    key: (String, u64),
    index: usize,
    children: &[Bounds<Pixels>],
    cx: &mut App,
) {
    if REVEALED.with(|held| held.borrow().as_ref() == Some(&key)) {
        return;
    }
    let (Some(item), view) = (children.get(index), handle.bounds()) else {
        return;
    };
    if view.size.height <= px(0.) {
        // The view hasn't been laid out yet: look again next frame.
        cx.refresh_windows();
        return;
    }
    let mut offset = handle.offset();
    if item.top() < view.top() {
        offset.y = offset.y + (view.top() - item.top());
    } else if item.bottom() > view.bottom() {
        offset.y = offset.y - (item.bottom() - view.bottom());
    }
    offset.y = offset.y.min(px(0.));
    handle.set_offset(offset);
    REVEALED.with(|held| *held.borrow_mut() = Some(key));
    cx.refresh_windows();
}

/// One repository of a review drawn inline (frame 68): its group row, then,
/// when it holds the picked file, its files bottom-up (leaf changes first),
/// the picked file's code right under its row. Other repositories are folded.
#[allow(clippy::too_many_arguments)]
pub(super) fn review_repo(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    documents: &Documents,
    session: &Session,
    diff: &RepoDiff,
    picked: Option<(&RepoDiff, &humata2_clients::model::FileDiff)>,
    cursor: Option<&humata2_review::ListCursor>,
    (font_size, line_height, cell_width): (f32, f32, f32),
    width: f32,
) -> AnyElement {
    let open = picked.is_some_and(|(held, _)| held.repo == diff.repo);
    // Frame 68: the repo row, its file and the file's code, 4 apart.
    let mut group = col().gap(px(4.)).child(ringed(
        review_repo_head(state, look, session, diff, open),
        at_repo(cursor, &diff.repo),
        look,
    ));
    if !open {
        return group.into_any_element();
    }
    let texts: Vec<String> = diff
        .files
        .iter()
        .map(|file| {
            file.lines
                .iter()
                .filter(|line| is_code(line))
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect();
    let ordered: Vec<OrderFile> = diff
        .files
        .iter()
        .zip(&texts)
        .map(|(file, text)| OrderFile { path: &file.path, text })
        .collect();
    // DS-26: the open file's head stays at the top of the review's view
    // while its lines scroll, and leaves with the file's last line.
    let mut pinned: Option<(usize, usize, (String, ScrollHandle, f32))> = None;
    let mut children = 1;
    for index in bottom_up(&ordered) {
        let file = &diff.files[index];
        let on = picked.is_some_and(|(held, f)| held.repo == diff.repo && f.path == file.path);
        let ring = at_file(cursor, &diff.repo, &file.path);
        if on {
            let name = format!("{}/{}", diff.repo, file.path);
            let handle = documents.session_diff_view(&diff.session, &name).handle;
            let shift = pinned_shift(&name, &handle);
            let head = div()
                .when(shift > 0., |head| head.relative().top(px(shift)).bg(rgb(look.panel)))
                .child(ringed(file_row(ui, state, look, diff, file, on, true), ring, look));
            group = group.child(if shift > 0. {
                deferred(head).into_any_element()
            } else {
                head.into_any_element()
            });
            pinned = Some((children, children + 1, (name, handle, shift)));
        } else {
            group = group.child(ringed(file_row(ui, state, look, diff, file, on, true), ring, look));
        }
        children += 1;
        if on {
            children += 1;
            group = group.child(review_code(
                ui,
                state,
                look,
                documents,
                session,
                diff,
                file,
                (font_size, line_height, cell_width),
                width,
            ));
        }
    }
    if let Some((head, code, (name, handle, shift))) = pinned {
        group = group.on_children_prepainted(move |bounds, _, _| {
            if let (Some(head), Some(code)) = (bounds.get(head), bounds.get(code)) {
                note_pinned(&name, &handle, shift, head, code);
            }
        });
    }
    group.into_any_element()
}

thread_local! {
    /// Per review file: where its head sits in the list's content when not
    /// pinned (its top, its height) and where its code ends.
    static PINNED: RefCell<HashMap<String, (f32, f32, f32)>> = RefCell::new(HashMap::new());
}

/// How far the open file's head is moved down so it stays at the top of the
/// review's view: none until its natural place scrolls off, and never past
/// the end of the file's code. Read from the last frame's place.
fn pinned_shift(name: &str, handle: &ScrollHandle) -> f32 {
    let Some((top, height, bottom)) = PINNED.with(|held| held.borrow().get(name).copied()) else {
        return 0.;
    };
    let scrolled = -f32::from(handle.offset().y);
    (scrolled - top).clamp(0., (bottom - height - top).max(0.))
}

/// Notes where the head and the code ended up this frame, in the list's own
/// coordinates, with the shift the head was drawn with taken back out.
fn note_pinned(
    name: &str,
    handle: &ScrollHandle,
    shift: f32,
    head: &Bounds<Pixels>,
    code: &Bounds<Pixels>,
) {
    let origin = f32::from(handle.bounds().origin.y) + f32::from(handle.offset().y);
    let top = f32::from(head.origin.y) - shift - origin;
    let bottom = f32::from(code.bottom()) - origin;
    PINNED.with(|held| {
        held.borrow_mut()
            .insert(name.to_owned(), (top, f32::from(head.size.height), bottom));
    });
}

/// The picked file's code in a review, with the agent's focus marked and
/// scrolled to.
#[allow(clippy::too_many_arguments)]
fn review_code(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    documents: &Documents,
    session: &Session,
    diff: &RepoDiff,
    file: &humata2_clients::model::FileDiff,
    (font_size, line_height, cell_width): (f32, f32, f32),
    width: f32,
) -> AnyElement {
    let avail = width - 16.;
    let name = format!("{}/{}", diff.repo, file.path);
    let scroll = documents.session_diff_view(&diff.session, &name);
    let LineFacts { digits, widest } = scroll.facts(&file.lines, line_facts);
    // Frame 68's code starts 36 in (wider only for longer numbers), the
    // number ending 8 before it. The sign keeps its own cell, taken from
    // the number column, so the number column is 36 less a cell. Metrics
    // takes the width with its 8 added. Three digits still fit in 36, so
    // the code starts where the design's does and the sign never overlaps it.
    let numbers = 36_f32.max(digits as f32 * cell_width + 6. + cell_width);
    let gutter_width = numbers - cell_width + 8.;
    let code_width = avail - numbers;
    let columns = ((code_width / cell_width).floor().max(4.)) as usize;
    // Scroll (DS-42): the number and sign stay put, only the code slides,
    // as far as the widest line needs.
    let max_shift = if state.review.wrap {
        0.
    } else {
        let start = code_start_of(gutter_width, cell_width, false, true);
        (widest as f32 * cell_width + cell_width) - (avail - start)
    };
    let shift = scroll.shift(max_shift);
    let sideways = {
        let scroll = scroll.clone();
        move |event: &ScrollEvent, _: &mut Window, cx: &mut App| {
            let delta = event.delta.pixel_delta(px(line_height));
            if delta.x != px(0.) && scroll.scroll_sideways(f32::from(delta.x), max_shift) {
                cx.refresh_windows();
            }
        }
    };
    let focus = state.pr_follower.focus.as_ref().filter(|focus| {
        focus.place.path == file.path
            && (focus.place.repo == diff.repo || focus.place.repo.rsplit('/').next() == Some(diff.repo.as_str()))
    });
    let rows = CodeRows::new(
        ui,
        state,
        documents,
        diff,
        file,
        Metrics {
            look,
            columns,
            gutter_width,
            cell_width,
            line_height,
            view_width: avail,
            review: true,
            old: false,
        },
        focus.and_then(|focus| focus.line().map(|line| (focus.place.side, line))),
        shift,
    );
    // Frame 68 draws each run of code lines as its own block and the cards
    // between the blocks, 4 apart; a card keeps the panel's line height.
    let block = || {
        col()
            .py(px(6.))
            .overflow_hidden()
            .font_family("Menlo")
            .text_size(px(font_size))
            .line_height(px(line_height))
            .bg(rgb(look.term))
    };
    let mut parts: Vec<AnyElement> = Vec::new();
    let mut lines: Vec<AnyElement> = Vec::new();
    // The part the focused line ends up in, so the focus scrolls to its block.
    let mut focus_part = None;
    for index in 0..file.lines.len() {
        let (line, cards) = rows.line(&file.lines, index);
        if rows.focus_index == Some(index) {
            focus_part = Some(parts.len());
        }
        lines.push(line);
        if !cards.is_empty() {
            parts.push(block().children(std::mem::take(&mut lines)).into_any_element());
            parts.extend(cards);
        }
    }
    if !lines.is_empty() {
        parts.push(block().children(lines).into_any_element());
    }
    let reveal = focus
        .zip(focus_part)
        .map(|(focus, part)| ((format!("{}|{name}", session.id), focus.seq), part));
    col()
        .gap(px(4.))
        .on_children_prepainted(move |children, _, cx| {
            if let Some((key, row)) = reveal.clone() {
                reveal_focus(&scroll.handle, key, row, &children, cx);
            }
        })
        .on_scroll_wheel(sideways)
        .id(SharedString::from(format!("change-code {name}")))
        .role(Role::Code)
        .aria_label(name)
        .test_support()
        .children(parts)
        .into_any_element()
}

/// A repository's group row in a review (frame 68): its name, its pull
/// request's number in the accent and the pull request's branch, dimmed.
pub(super) fn review_repo_head(
    state: &AppState,
    look: Look,
    session: &Session,
    diff: &RepoDiff,
    open: bool,
) -> AnyElement {
    let number = humata2_review::comments::github_target(&state.workspace, &session.id, &diff.repo)
        .map(|(_, number)| number);
    let branch = session
        .clones
        .iter()
        .find(|clone| clone.repo == diff.repo || clone.repo.rsplit('/').next() == Some(diff.repo.as_str()))
        .map_or("", |clone| clone.branch.as_str());
    let name = diff.repo.rsplit('/').next().unwrap_or(diff.repo.as_str());
    row()
        .id(SharedString::from(format!("review-repo-{}", diff.repo)))
        .test_support()
        .gap(px(6.))
        .h(px(26.))
        .px(px(4.))
        .child(icon(
            if open { IconName::ChevronDown } else { IconName::ChevronRight },
            look.tertiary,
            12.,
        ))
        .child(icon(IconName::FolderGit2, look.secondary, 12.))
        .child(text(name.to_owned()).font_weight(FontWeight::SEMIBOLD))
        .children(number.map(|number| text(format!("#{number}")).text_color(rgb(look.accent))))
        .child({
            let shown = div()
                .id(SharedString::from(format!("review-repo-branch-{}", diff.repo)))
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(11.))
                .text_color(rgb(look.tertiary))
                .child(branch.to_owned());
            // No branch means nothing to name: an empty tooltip would only
            // cover the free space beside the repo.
            if branch.is_empty() {
                shown
            } else {
                full_text(shown, branch.to_owned())
            }
        })
        .into_any_element()
}

/// What the Diff panel says when nothing is picked and nothing failed: the
/// base decides the words.
/// The base the chip names while no diff has said which branch it is.
pub(super) fn picked_base_name(state: &AppState, base: &DiffBase) -> String {
    // D800: a stack layer or the whole stack comes before every plain base.
    if let Some(scope) = stack_scope(state, base) {
        return scope.name().to_owned();
    }
    match base {
        DiffBase::OriginMain => "origin/main".to_owned(),
        DiffBase::UncommittedOnly => "uncommitted only".to_owned(),
        DiffBase::SinceLastReviewed => "your last review".to_owned(),
        DiffBase::SinceFork => "the fork".to_owned(),
        DiffBase::OtherSession(id) => state
            .workspace
            .session(id)
            .map_or_else(|| "another session".to_owned(), |other| other.title.clone()),
        DiffBase::PullRequestBase => "the pull request's base".to_owned(),
        DiffBase::Spec(_) => "each repo's base".to_owned(),
    }
}

/// What a stack shows in the Diff (D800): one layer, or the whole stack.
#[derive(Clone, Copy)]
pub(super) enum StackScope {
    Layer,
    Whole,
}

impl StackScope {
    /// What the base chip names.
    fn name(self) -> &'static str {
        match self {
            Self::Layer => "this layer",
            Self::Whole => "the whole stack",
        }
    }

    /// What the panel says when nothing changed there.
    pub(super) fn nothing(self) -> &'static str {
        match self {
            Self::Layer => "No changes in this layer",
            Self::Whole => "No changes in the stack",
        }
    }
}

/// Which part of a stack the open session's Diff shows, when it shows one: the
/// base is a stack's own (a spec whose bases are a range or an overlay), or the
/// session's plan carries a stack, which picks its bases itself (DS-62).
pub(super) fn stack_scope(state: &AppState, base: &DiffBase) -> Option<StackScope> {
    let own = matches!(
        base,
        DiffBase::Spec(specs) if specs.iter().any(|spec| {
            matches!(spec.base, RepoBase::Range { .. } | RepoBase::Overlay { .. })
        })
    );
    if !own && !crate::panel_stack::hides_base_picker(&state.review) {
        return None;
    }
    let layer = state
        .session
        .open
        .as_ref()
        .and_then(|id| state.review.layer_of(id));
    Some(if layer.is_some() { StackScope::Layer } else { StackScope::Whole })
}

/// A transparent layer over the window behind the base menu: a click on it
/// closes the menu and reaches nothing else, as the Desk pop-ups' scrim does.
pub(super) fn base_scrim(ui: &Rc<Ui>) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id("diff-base-scrim")
        .absolute()
        .top(px(-4000.))
        .left(px(-4000.))
        .w(px(8000.))
        .h(px(8000.))
        .occlude()
        .on_click(move |_, _, cx| ui.send(AppAction::Review(ReviewAction::CloseBaseMenu), cx))
}

/// The layer behind the Push & PR menu: a click on it closes the menu and
/// reaches nothing else.
pub(super) fn push_scrim(ui: &Rc<Ui>) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id("changes-push-scrim")
        .absolute()
        .top(px(-4000.))
        .left(px(-4000.))
        .w(px(8000.))
        .h(px(8000.))
        .occlude()
        .on_click(move |_, _, cx| ui.send(AppAction::Review(ReviewAction::ClosePushMenu), cx))
        .test_support()
}

/// The Push & PR menu above the footer: one row per repository with its own
/// push button, which pushes that repository only.
pub(super) fn push_menu(ui: &Rc<Ui>, state: &AppState, look: Look, diffs: &[&RepoDiff]) -> impl IntoElement {
    col()
        .id("changes-push-list")
        .role(Role::ListBox)
        .aria_label("Push & PR")
        .absolute()
        .bottom(px(28.))
        .left(px(0.))
        .w(px(280.))
        .p(px(4.))
        .gap(px(1.))
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
        .occlude()
        .text_size(px(12.5))
        .children(diffs.iter().map(|diff| {
            row()
                .gap(px(8.))
                .h(px(28.))
                .px(px(8.))
                .child(full_text(
                    text(repo_label(&diff.repo).to_owned())
                        .id(SharedString::from(format!("push-list-repo-{}", diff.repo)))
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis(),
                    repo_label(&diff.repo).to_owned(),
                ))
                .child(div().flex_1())
                .child(push_control(ui, state, look, &diff.repo))
        }))
        .test_support()
}

/// One base in the menu: its label, a tick when it is the current base, and
/// the action a click sends.
fn base_row(
    ui: &Rc<Ui>,
    id: SharedString,
    label: impl Into<SharedString>,
    current: bool,
    event: AppAction,
    look: Look,
) -> impl IntoElement {
    let ui = ui.clone();
    let label: SharedString = label.into();
    row()
        .id(id)
        .role(Role::ListBoxOption)
        .aria_label(label.clone())
        .aria_selected(current)
        .gap(px(8.))
        .h(px(26.))
        .px(px(8.))
        .rounded(px(5.))
        .cursor_pointer()
        .hover(move |x| x.bg(rgb(look.fill)))
        .child(
            div()
                .w(px(12.))
                .flex_shrink_0()
                .children(current.then(|| icon(IconName::Check, look.accent, 12.))),
        )
        .child(label)
        .on_click(move |_, _, cx| ui.send(event.clone(), cx))
        .test_support()
}

/// The base menu under the Diff header (RV-1, RV-6): what the changes are
/// compared with, the current base ticked, then Mark reviewed. The fork
/// base shows only for a fork; other sessions of the channel follow.
pub(super) fn base_menu(ui: &Rc<Ui>, state: &AppState, look: Look) -> impl IntoElement {
    let here = state
        .session
        .open
        .as_ref()
        .and_then(|id| state.workspace.session(id));
    let current = state
        .session
        .open
        .as_ref()
        .map(|id| state.review.targets_of(id).preset)
        .unwrap_or_default();
    let pick = |id: &'static str, label: &'static str, base: DiffBase| {
        let on = base == current;
        base_row(
            ui,
            SharedString::from(id),
            label,
            on,
            AppAction::Review(ReviewAction::PickBase(base)),
            look,
        )
        .into_any_element()
    };
    let mut items = vec![
        pick(
            "diff-base-origin-main",
            "origin/main (where the branch started)",
            DiffBase::OriginMain,
        ),
        pick(
            "diff-base-uncommitted",
            "Uncommitted only",
            DiffBase::UncommittedOnly,
        ),
        pick(
            "diff-base-since-reviewed",
            "Since I last reviewed",
            DiffBase::SinceLastReviewed,
        ),
    ];
    if here.is_some_and(|session| session.pull_request_review) {
        items.insert(
            0,
            pick(
                "diff-base-pull-request",
                "The pull request's base",
                DiffBase::PullRequestBase,
            ),
        );
    }
    if here.is_some_and(|session| session.parent.is_some()) {
        items.push(pick(
            "diff-base-since-fork",
            "Since the fork from its parent",
            DiffBase::SinceFork,
        ));
    }
    let others: Vec<&Session> = here.map_or_else(Vec::new, |here| {
        state
            .workspace
            .sessions
            .iter()
            .filter(|other| {
                other.channel == here.channel && other.id != here.id && !other.archived
            })
            .collect()
    });
    if !others.is_empty() {
        items.push(
            secondary(look, "Another session's branch")
                .px(px(8.))
                .pt(px(6.))
                .into_any_element(),
        );
    }
    for other in others {
        let base = DiffBase::OtherSession(other.id.clone());
        let on = base == current;
        items.push(
            base_row(
                ui,
                SharedString::from(format!("diff-base-other-{}", other.id)),
                format!("{}'s branch", other.title),
                on,
                AppAction::Review(ReviewAction::PickBase(base)),
                look,
            )
            .into_any_element(),
        );
    }
    items.push(
        div()
            .h(px(1.))
            .my(px(4.))
            .bg(rgb(look.line))
            .into_any_element(),
    );
    let mark = ui.clone();
    items.push(
        row()
            .id("diff-mark-reviewed")
            .role(Role::Button)
            .aria_label("Mark reviewed")
            .gap(px(8.))
            .h(px(26.))
            .px(px(8.))
            .rounded(px(5.))
            .cursor_pointer()
            .hover(move |x| x.bg(rgb(look.fill)))
            .child(div().w(px(12.)).flex_shrink_0())
            .child("Mark reviewed")
            .on_click(move |_, _, cx| {
                mark.send(AppAction::Review(ReviewAction::MarkReviewed), cx)
            })
            .test_support()
            .into_any_element(),
    );
    col()
        .id("diff-base-menu")
        .role(Role::ListBox)
        .aria_label("Compare with")
        .absolute()
        .top(px(28.))
        .right(px(0.))
        .w(px(260.))
        .p(px(4.))
        .gap(px(1.))
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
        .occlude()
        .text_size(px(12.5))
        .children(items)
        .test_support()
}

/// A repository's one row in the Diff panel (DS-5): the chevron that folds
/// its files, its name, its clone's branch when that isn't the session's, the
/// base it is compared with when that differs from the header's, and its
/// totals at the right. Push & PR lives in the footer.
#[allow(clippy::too_many_arguments)]
pub(super) fn repo_head(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    documents: &Documents,
    index: usize,
    diff: &RepoDiff,
    branch: &str,
    folded: bool,
) -> AnyElement {
    let (added, removed) = diff.files.iter().fold((0_u64, 0_u64), |(a, r), file| {
        (a + u64::from(file.added), r + u64::from(file.removed))
    });
    let own = state
        .workspace
        .session(&diff.session)
        .and_then(|session| {
            session.clones.iter().find(|clone| {
                clone.repo == diff.repo || clone.repo.rsplit('/').next() == Some(diff.repo.as_str())
            })
        })
        .map(|clone| clone.branch.clone())
        .filter(|own| !own.is_empty() && own != branch);
    let toggle = ui.clone();
    let repo = diff.repo.clone();
    let label = if folded { "Expand" } else { "Collapse" };
    // One line: when space runs out the branch gives way first, then the
    // base chip, then the repo name; the totals never shrink.
    row()
        .id(SharedString::from(format!("repo-row-{index}")))
        .gap(px(6.))
        .h(px(26.))
        .px(px(4.))
        .pr(px(8.))
        .min_w_0()
        .whitespace_nowrap()
        .child(
            Button::new(SharedString::from(format!("repo-toggle-{index}")))
                .ghost()
                .small()
                .w(px(20.))
                .h(px(20.))
                .flex_shrink_0()
                .icon(icon(
                    if folded { IconName::ChevronRight } else { IconName::ChevronDown },
                    look.tertiary,
                    12.,
                ))
                .accessibility_label(format!("{label} {}", diff.repo))
                .on_click(move |_, _, cx| {
                    toggle.send(
                        AppAction::Review(ReviewAction::ToggleRepo { repo: repo.clone() }),
                        cx,
                    )
                }),
        )
        .child(full_text(
            text(repo_label(&diff.repo).to_owned())
                .id(SharedString::from(format!("repo-name-{index}")))
                .font_weight(FontWeight::SEMIBOLD)
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis(),
            repo_label(&diff.repo).to_owned(),
        ))
        .children(own.map(|own| {
            full_text(
                secondary(look, own.clone())
                    .id(SharedString::from(format!("repo-branch-{index}")))
                    .test_support()
                    .text_color(rgb(look.tertiary))
                    .min_w_0()
                    .flex_shrink(8.)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis(),
                own,
            )
        }))
        .child(repo_base(ui, state, look, documents, index, diff))
        .child(div().flex_1())
        .child(
            row()
                .id(SharedString::from(format!("repo-totals-{index}")))
                .gap(px(6.))
                .flex_shrink_0()
                .whitespace_nowrap()
                .child(
                    text(format!("+{added}"))
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .text_size(px(11.))
                        .text_color(rgb(GREEN)),
                )
                .child(
                    text(format!("−{removed}"))
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .text_size(px(11.))
                        .text_color(rgb(RED)),
                )
                .test_support(),
        )
        .test_support()
        .into_any_element()
}

/// A repository's "against X" chip and, while it is open, its menu.
pub(super) fn repo_base(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    documents: &Documents,
    index: usize,
    diff: &RepoDiff,
) -> Div {
    let picker = state
        .review
        .repo_picker
        .as_ref()
        .filter(|picker| picker.repo == diff.repo);
    let open = picker.is_some();
    // Another session's branch has no per-repository form: the base shows,
    // but only the header's menu changes it (VIEWS P1).
    let fixed = state.session.open.as_ref().is_some_and(|id| {
        matches!(state.review.targets_of(id).preset, DiffBase::OtherSession(_))
    });
    // While every repository shares the header's base, the chip shows only
    // its chevron: the words are the header's (D510, amended by D739).
    let words = fixed
        || state
            .session
            .open
            .as_ref()
            .is_some_and(|id| state.review.compares_per_repo(id));
    let tone = if open { look.accent } else { look.secondary };
    let toggle = ui.clone();
    let repo = diff.repo.clone();
    let id = SharedString::from(format!("repo-base-{index}"));
    let spoken = format!("{}: against {}", diff.repo, diff.base);
    let pick = move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
        toggle.send(
            AppAction::Review(ReviewAction::ToggleRepoBase { repo: repo.clone() }),
            cx,
        )
    };
    let against = if words {
        row()
            .id(id)
            .role(Role::Button)
            .aria_label(spoken)
            .aria_expanded(open)
            .text_size(px(11.5))
            .whitespace_nowrap()
            .min_w_0()
            .child(text("against").flex_shrink_0().mr(px(3.)).text_color(rgb(tone)))
            // A long base (the daemon's "not reviewed yet: …") ends in
            // an ellipsis rather than pushing Push & PR out of the panel.
            .child(
                text(diff.base.clone())
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(rgb(if open { look.accent } else { look.text })),
            )
            .when(!fixed, |chip| {
                chip.cursor_pointer()
                    .child(
                        div()
                            .flex_shrink_0()
                            .ml(px(3.))
                            .child(icon(IconName::ChevronsUpDown, tone, 12.)),
                    )
                    .on_click(pick)
            })
            .test_support()
            .into_any_element()
    } else {
        // The chevron alone: a small icon button, so it keeps a target, a
        // hover fill, a focus ring and a tooltip that says what it changes.
        Quiet {
            button: Button::new(id)
                .small()
                .w(px(24.))
                .h(px(20.))
                .flex_shrink_0()
                .icon(icon(IconName::ChevronsUpDown, tone, 12.))
                .tooltip(format!("{}: against {} · change", diff.repo, diff.base))
                .accessibility_label(spoken)
                .on_click(pick),
            color: tone,
            fill: look.fill,
        }
        .into_any_element()
    };
    // Not clipped: the menu below is a deferred child that must draw past it.
    // The chip's text ellipsizes, and it gives way before the repo name.
    div().relative().min_w_0().flex_shrink(4.).child(against).when_some(picker, |wrap, picker| {
        // Deferred, so the menu paints over the panel and takes the clicks
        // there; the scrim first, then the menu.
        wrap.child(deferred(repo_base_scrim(ui)))
            .child(deferred(repo_base_menu(ui, state, look, documents, picker)))
    })
}

/// The layer behind a repository's base menu: a click on it closes the menu
/// and reaches nothing else.
fn repo_base_scrim(ui: &Rc<Ui>) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id("repo-base-scrim")
        .absolute()
        .top(px(-4000.))
        .left(px(-4000.))
        .w(px(8000.))
        .h(px(8000.))
        .occlude()
        .on_click(move |_, _, cx| ui.send(AppAction::Review(ReviewAction::CloseRepoBase), cx))
        .test_support()
}

/// One repository's base menu (frame 86, VIEWS P3): what to compare with,
/// the branches its clone knew at the last fetch under a filter, and the
/// branch or commit typed into the filter.
fn repo_base_menu(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    documents: &Documents,
    picker: &RepoPicker,
) -> impl IntoElement {
    let Some(session) = state.session.open.clone() else {
        return col().id("repo-base-menu").test_support();
    };
    let current = state.review.targets_of(&session).of(&picker.repo);
    let pick = |id: SharedString, label: String, base: RepoBase| {
        let on = base == current;
        base_row(
            ui,
            id,
            label,
            on,
            AppAction::Review(ReviewAction::PickRepoBase {
                session: session.clone(),
                repo: picker.repo.clone(),
                base,
            }),
            look,
        )
        .into_any_element()
    };
    let heading = |words: &'static str| {
        secondary(look, words)
            .px(px(8.))
            .pt(px(4.))
            .pb(px(2.))
            .into_any_element()
    };
    let line = || div().h(px(1.)).my(px(4.)).bg(rgb(look.line)).into_any_element();
    let mut items = vec![
        heading("Compare with"),
        pick("repo-base-default".into(), "Default branch (origin/HEAD)".into(), RepoBase::DefaultBranch),
        pick("repo-base-uncommitted".into(), "Uncommitted only".into(), RepoBase::Uncommitted),
        pick("repo-base-since-reviewed".into(), "Since I last reviewed".into(), RepoBase::LastReviewed),
    ];
    if state
        .workspace
        .session(&session)
        .is_some_and(|here| here.parent.is_some())
    {
        items.push(pick(
            "repo-base-since-fork".into(),
            "Since the fork from its parent".into(),
            RepoBase::ForkPoint,
        ));
    }
    if state
        .workspace
        .session(&session)
        .is_some_and(|here| here.pull_request_review)
    {
        items.insert(
            1,
            pick(
                "repo-base-pr".into(),
                "Pull request's base".into(),
                RepoBase::PullRequestBase,
            ),
        );
    }
    items.push(line());
    items.push(heading("Branches · as of last fetch"));
    items.push(
        div()
            .id("repo-base-filter")
            .test_support()
            .px(px(2.))
            .pb(px(4.))
            .child(
                gpui_kit::component::input::Input::new(&documents.base_filter())
                    .aria_label("Filter branches")
                    .small()
                    .min_h(px(26.))
                    .bg(rgb(look.card))
                    .border_color(rgb(look.line))
                    .text_color(rgb(look.text)),
            )
            .into_any_element(),
    );
    match &picker.bases {
        None => items.push(
            secondary(look, "Reading branches…")
                .px(px(8.))
                .py(px(4.))
                .id("repo-base-loading")
                .test_support()
                .into_any_element(),
        ),
        Some(Err(error)) => {
            let retry = ui.clone();
            items.push(
                col()
                    .id("repo-base-failed")
                    .aria_label(error.clone())
                    .test_support()
                    .gap(px(4.))
                    .px(px(8.))
                    .py(px(4.))
                    .child(secondary(look, error.clone()))
                    .child(
                        Button::new("repo-base-retry")
                            .small()
                            .label("Try again")
                            .on_click(move |_, _, cx| {
                                retry.send(AppAction::Review(ReviewAction::ReloadRepoBases), cx)
                            }),
                    )
                    .into_any_element(),
            );
        }
        Some(Ok(_)) => {
            let branches = picker.branches();
            if branches.is_empty() {
                items.push(
                    secondary(look, "No branch matches")
                        .px(px(8.))
                        .py(px(4.))
                        .id("repo-base-no-match")
                        .test_support()
                        .into_any_element(),
                );
            }
            for (n, branch) in branches.into_iter().enumerate() {
                items.push(pick(
                    SharedString::from(format!("repo-base-pick-{n}")),
                    branch.to_owned(),
                    RepoBase::Branch(branch.to_owned()),
                ));
            }
        }
    }
    items.push(line());
    // The filter's words as a branch, tag or commit; a name that doesn't
    // resolve fails in the read that follows, with a fix (VIEWS P3).
    let typed = picker.filter.trim().to_owned();
    if typed.is_empty() {
        items.push(
            row()
                .id("repo-base-rev")
                .aria_label("Branch or commit: type it in the filter")
                .test_support()
                .gap(px(8.))
                .h(px(26.))
                .px(px(8.))
                .child(div().w(px(12.)).flex_shrink_0())
                .child(text("Branch or commit…").text_color(rgb(look.secondary)))
                .into_any_element(),
        );
    } else {
        items.push(pick(
            "repo-base-rev".into(),
            format!("Branch or commit “{typed}”"),
            RepoBase::Rev(typed),
        ));
    }
    col()
        .id("repo-base-menu")
        .role(Role::ListBox)
        .aria_label(format!("Compare {} with", picker.repo))
        .absolute()
        .top(px(24.))
        // RV-7, DS-50: anchored by its right edge to the chip, so a chip near
        // the panel's edge never pushes the menu past it.
        .right(px(0.))
        .w(px(240.))
        .p(px(6.))
        .gap(px(1.))
        .rounded(px(9.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
        .occlude()
        .text_size(px(12.5))
        .children(items)
        .test_support()
}

/// A comment the workspace carries, already sent.
/// The Push & PR control of one repository group, and what its last push
/// left beside it: the pull request, or why the push failed (RV-2).
fn push_control(ui: &Rc<Ui>, state: &AppState, look: Look, repo: &str) -> Div {
    let outcome = state
        .session
        .open
        .clone()
        .and_then(|session| state.review.pushes.get(&(session, repo.to_owned())));
    let pushing = outcome == Some(&PushState::Pushing);
    let ui_push = ui.clone();
    let repo_push = repo.to_owned();
    let button = Button::new(SharedString::from(format!("changes-push-{repo}")))
        .small()
        .label(if pushing { "Pushing…" } else { "Push & PR" })
        // Every repo group draws the same words, so the accessible name says
        // which repo the button pushes (D388); the drawn text is unchanged.
        .accessibility_label(format!("Push & PR: {repo}"))
        .disabled(pushing)
        .on_click(move |_, _, cx| {
            ui_push.send(
                AppAction::Review(ReviewAction::Push {
                    repo: repo_push.clone(),
                }),
                cx,
            )
        });
    let status = match outcome {
        Some(PushState::Pushed { pull_request, note }) => {
            let label = pull_request.map_or_else(|| "Pushed".to_owned(), |number| format!("PR #{number}"));
            // A pull request left on another base says so (VIEWS P5); the
            // whole note is its accessible name.
            let spoken = if note.is_empty() { label.clone() } else { note.clone() };
            Some(
                row()
                    .id(SharedString::from(format!("changes-pushed-{repo}")))
                    .aria_label(spoken)
                    .test_support()
                    .gap(px(4.))
                    .child(
                        text(label)
                            .text_size(px(11.))
                            .text_color(rgb(look.secondary)),
                    )
                    .when(!note.is_empty(), |status| {
                        status.child(
                            div()
                                .id(SharedString::from(format!("changes-push-note-{repo}")))
                                .test_support()
                                .child(icon(IconName::Info, look.secondary, 12.)),
                        )
                    })
                    .into_any_element(),
            )
        }
        Some(PushState::Failed(reason)) => Some(
            row()
                .id(SharedString::from(format!("changes-push-failed-{repo}")))
                .aria_label(reason.clone())
                .test_support()
                .child(
                    text("Push failed")
                        .text_size(px(11.))
                        .text_color(rgb(RED)),
                )
                .into_any_element(),
        ),
        Some(PushState::Pushing) | None => None,
    };
    row()
        .gap(px(6.))
        .flex_shrink_0()
        .child(Quiet { button, color: look.accent, fill: look.fill })
        .when_some(status, |control, status| control.child(status))
}
