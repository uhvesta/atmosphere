//! Desktop `Reviews` inbox page (DECISIONS.md §1):
//! grouped into **Local · ready to push** and **On GitHub · feedback & review requests**
//! (including manually watched PRs), with stripped row noise and single-click row open.

use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::Button;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route};
use humata2_clients::model::{MachineKind, SessionId};
use humata2_pr_review::dialog::DialogAction;
use humata2_todo::AskKind;
use humata2_todo_feature::TodoAction;

use crate::{chrome, theme::Look};
use humata2_ui_core::bridge::Ui;
use humata2_ui_core::widgets::{col, row};

/// Which section of the Reviews inbox a row belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewGroup {
    LocalReadyToPush,
    OnGithub,
}

/// Concise status chip tone for a review row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewChipTone {
    Amber,
    Blue,
    Neutral,
    Red,
}

/// Concise status chip shown on the right of a review row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewChip {
    pub label: String,
    pub tone: ReviewChipTone,
}

/// Action performed when a review row is clicked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewRowAction {
    OpenSession(SessionId),
    StartTodoReview(String),
    OpenPrUrl(String),
}

/// One normalized row in the Reviews inbox.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewRow {
    pub id: String,
    pub group: ReviewGroup,
    pub key: Option<String>,
    pub title: String,
    pub context_meta: Option<String>,
    pub detail_line: Option<String>,
    pub chip: ReviewChip,
    pub age: Option<String>,
    pub action: ReviewRowAction,
}

/// Grouped rows and total count for the Reviews inbox page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReviewsInboxData {
    pub local: Vec<ReviewRow>,
    pub github: Vec<ReviewRow>,
    pub total: usize,
}

/// Parses a GitHub PR URL or shorthand (`owner/repo#123`) into `(owner, repo, number)`.
#[must_use]
pub fn parse_watch_pr_url(raw: &str) -> Option<(String, String, u64)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // URL: https://github.com/owner/repo/pull/123[/files...]
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    if let Some(path) = without_scheme
        .strip_prefix("github.com/")
        .or_else(|| without_scheme.strip_prefix("www.github.com/"))
    {
        let path = path.split(['?', '#']).next().unwrap_or(path);
        let mut parts = path.split('/').filter(|s| !s.is_empty());
        let owner = parts.next()?;
        let repo = parts.next()?;
        let kind = parts.next()?;
        if kind != "pull" {
            return None;
        }
        let number = parts.next()?.parse::<u64>().ok()?;
        return Some((owner.to_owned(), repo.to_owned(), number));
    }
    // Shorthand: owner/repo#123
    if let Some((repo_part, num_part)) = trimmed.split_once('#') {
        let (owner, repo) = repo_part.split_once('/')?;
        if owner.is_empty() || repo.is_empty() || repo.contains('/') {
            return None;
        }
        let number = num_part.parse::<u64>().ok()?;
        return Some((owner.to_owned(), repo.to_owned(), number));
    }
    None
}

/// Formats stack layer info, returning `None` for single-PR / 1-layer reviews
/// so `"1 layer"` noise is never shown (DECISIONS.md §1).
#[must_use]
pub fn format_layer_summary(layers: usize, names: &[&str]) -> Option<String> {
    if layers <= 1 {
        return None;
    }
    let clean_names: Vec<&str> = names
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if clean_names.is_empty() {
        Some(format!("{layers} layers"))
    } else {
        Some(format!("{layers} layers · {}", clean_names.join(", ")))
    }
}

/// Shortens verbose review status chip copy into concise labels (DECISIONS.md §1).
#[must_use]
pub fn concise_chip_label(raw: &str) -> String {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("fixes pushed") {
        "Fixes pushed".to_owned()
    } else if lower.starts_with("new feedback") || lower.contains("feedback since") {
        "New feedback".to_owned()
    } else if lower.starts_with("requested your review") || lower == "review requested" {
        "Review requested".to_owned()
    } else if lower.contains("conflict") {
        "Conflicts".to_owned()
    } else if lower.is_empty() {
        "New".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Drops noisy boilerplate phrases such as `"no To-do, not your work"`.
#[must_use]
pub fn strip_row_noise(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower == "no to-do, not your work" || lower.contains("no to-do, not your work") {
        return None;
    }
    Some(trimmed.to_owned())
}

/// Builds the grouped Reviews inbox model from [`AppState`].
#[must_use]
pub fn build_reviews_inbox(state: &AppState) -> ReviewsInboxData {
    let mut local = Vec::new();
    let mut github = Vec::new();
    let mut seen_sessions: HashSet<SessionId> = HashSet::new();
    let mut seen_prs: HashSet<String> = HashSet::new();

    for entry in &state.todo.items {
        let todo = &entry.todo;
        if todo.closed.is_some() || !todo.state.is_active() {
            continue;
        }
        let channel_name = todo.channel.as_deref().map(|ch| {
            let resolved = state.todo.channel_name(ch);
            format!("#{resolved}")
        });

        // 1. Local · ready to push: active To-dos with an agent ReviewRequest ask.
        if let Some(ask) = &todo.ask
            && let AskKind::ReviewRequest { summary, round, .. } = &ask.kind
        {
            let session_id = todo
                .sessions
                .first()
                .map(|id| SessionId(id.clone()));
            if let Some(ref sid) = session_id {
                seen_sessions.insert(sid.clone());
            }
            let chip = if *round > 1 {
                ReviewChip {
                    label: concise_chip_label("Fixes pushed since you looked"),
                    tone: ReviewChipTone::Amber,
                }
            } else {
                ReviewChip {
                    label: "New".to_owned(),
                    tone: ReviewChipTone::Blue,
                }
            };
            let detail_line = strip_row_noise(summary);
            let age = (todo.ask_detail.asked_minutes_ago > 0)
                .then(|| format!("{}m", todo.ask_detail.asked_minutes_ago));
            let action = match session_id {
                Some(sid) => ReviewRowAction::OpenSession(sid),
                None => ReviewRowAction::StartTodoReview(todo.key.clone()),
            };
            local.push(ReviewRow {
                id: format!("local-todo-{}", todo.key),
                group: ReviewGroup::LocalReadyToPush,
                key: (!todo.key.is_empty()).then(|| todo.key.clone()),
                title: todo.title.clone(),
                context_meta: channel_name.clone(),
                detail_line,
                chip,
                age,
                action,
            });
        }

        // 2. On GitHub · review requests attached to a To-do.
        if let Some(review) = &todo.review {
            for pr in &review.repo_prs {
                seen_prs.insert(pr.clone());
            }
            let pr_list = review.repo_prs.join(" · ");
            let meta = match (pr_list.is_empty(), review.author.trim().is_empty()) {
                (false, false) => Some(format!("{pr_list} · by {}", review.author.trim())),
                (false, true) => Some(pr_list),
                (true, false) => Some(format!("by {}", review.author.trim())),
                (true, true) => channel_name.clone(),
            };
            let layer_summary = format_layer_summary(review.repo_prs.len(), &[]);
            let detail_line = match layer_summary {
                Some(layers) => Some(format!("{layers} · Requested your review on GitHub")),
                None => Some("Requested your review on GitHub".to_owned()),
            };
            let age = (review.waiting_days > 0).then(|| format!("{}d", review.waiting_days));
            github.push(ReviewRow {
                id: format!("gh-review-{}", todo.key),
                group: ReviewGroup::OnGithub,
                key: (!todo.key.is_empty()).then(|| todo.key.clone()),
                title: todo.title.clone(),
                context_meta: meta,
                detail_line,
                chip: ReviewChip {
                    label: concise_chip_label("Review requested"),
                    tone: ReviewChipTone::Neutral,
                },
                age,
                action: ReviewRowAction::StartTodoReview(todo.key.clone()),
            });
        }

        // 3. On GitHub · watched PRs on active To-dos (including WatchingPr state).
        for pr in &todo.watched_prs {
            if pr.merged || pr.closed_without_merge {
                continue;
            }
            if !seen_prs.insert(pr.repo_pr.clone()) {
                continue;
            }
            let (chip_label, tone) = if pr.has_conflicts {
                (concise_chip_label("Merge conflicts"), ReviewChipTone::Red)
            } else if pr.unresolved_comments > 0 {
                (concise_chip_label("New feedback since you looked"), ReviewChipTone::Amber)
            } else {
                ("Watched".to_owned(), ReviewChipTone::Neutral)
            };
            let mut details = Vec::new();
            if pr.unresolved_comments > 0 {
                let word = if pr.unresolved_comments == 1 {
                    "inline comment"
                } else {
                    "inline comments"
                };
                details.push(format!("{} {word}", pr.unresolved_comments));
            }
            if !pr.ci_state.trim().is_empty() {
                details.push(format!("CI {}", pr.ci_state.trim()));
            }
            let detail_line = (!details.is_empty()).then(|| details.join(" · "));
            let title = if pr.title.trim().is_empty() {
                todo.title.clone()
            } else {
                pr.title.clone()
            };
            let action = parse_watch_pr_url(&pr.repo_pr)
                .map(|(owner, repo, number)| {
                    ReviewRowAction::OpenPrUrl(format!(
                        "https://github.com/{owner}/{repo}/pull/{number}"
                    ))
                })
                .unwrap_or_else(|| ReviewRowAction::StartTodoReview(todo.key.clone()));
            github.push(ReviewRow {
                id: format!("gh-watched-{}", pr.repo_pr),
                group: ReviewGroup::OnGithub,
                key: (!todo.key.is_empty()).then(|| todo.key.clone()),
                title,
                context_meta: Some(pr.repo_pr.clone()),
                detail_line,
                chip: ReviewChip {
                    label: chip_label,
                    tone,
                },
                age: None,
                action,
            });
        }
    }

    // 4. Open PR review sessions in the workspace not already represented above.
    for session in &state.workspace.sessions {
        if session.archived || !session.pull_request_review {
            continue;
        }
        if !seen_sessions.insert(session.id.clone()) {
            continue;
        }
        let channel = state
            .workspace
            .channel(&session.channel)
            .map(|c| format!("#{}", c.name));
        let moved = state.pr_heads.notice(&session.id);
        let has_new_commits = moved.iter().any(|(_, m)| m.new_commits > 0 || m.force_pushed);
        let chip = if has_new_commits {
            ReviewChip {
                label: concise_chip_label("Fixes pushed since you looked"),
                tone: ReviewChipTone::Amber,
            }
        } else {
            ReviewChip {
                label: "Reviewing".to_owned(),
                tone: ReviewChipTone::Blue,
            }
        };
        github.push(ReviewRow {
            id: format!("gh-session-{}", session.id.0),
            group: ReviewGroup::OnGithub,
            key: None,
            title: session.shown_title(),
            context_meta: channel,
            detail_line: None,
            chip,
            age: None,
            action: ReviewRowAction::OpenSession(session.id.clone()),
        });
    }

    let total = local.len() + github.len();
    ReviewsInboxData {
        local,
        github,
        total,
    }
}

fn chip_colors(tone: ReviewChipTone, dark: bool) -> (u32, u32) {
    match (tone, dark) {
        (ReviewChipTone::Amber, false) => (0xfff4e5, 0xb45309),
        (ReviewChipTone::Amber, true) => (0x3a2810, 0xffb340),
        (ReviewChipTone::Blue, false) => (0xe8f1ff, 0x1d4ed8),
        (ReviewChipTone::Blue, true) => (0x162844, 0x60a5fa),
        (ReviewChipTone::Red, false) => (0xfee2e2, 0xb91c1c),
        (ReviewChipTone::Red, true) => (0x3d1919, 0xf87171),
        (ReviewChipTone::Neutral, false) => (0xf3f4f6, 0x4b5563),
        (ReviewChipTone::Neutral, true) => (0x2a2a2e, 0xd1d5db),
    }
}

fn open_watch_pr_action(state: &AppState) -> AppAction {
    let machine = state
        .workspace
        .machines
        .iter()
        .find(|m| m.online && matches!(m.kind, MachineKind::ThisMac))
        .or_else(|| state.workspace.machines.iter().find(|m| m.online))
        .map(|m| m.id.clone());
    let agent = state
        .default_agent(None)
        .unwrap_or(humata2_clients::model::Agent::Claude);
    AppAction::PrDialog(DialogAction::Open { machine, agent })
}

fn draw_row(
    ui: &Rc<Ui>,
    state: &AppState,
    item: &ReviewRow,
    show_divider: bool,
    look: Look,
) -> AnyElement {
    let dark = look.text > look.content;
    let (chip_bg, chip_fg) = chip_colors(item.chip.tone, dark);
    let ui = ui.clone();
    let action = item.action.clone();
    let watch_fallback = open_watch_pr_action(state);

    let mut title_line = row().gap(px(6.)).items_center().min_w_0();
    if let Some(key) = &item.key {
        title_line = title_line.child(
            div()
                .flex_shrink_0()
                .text_size(px(12.))
                .text_color(rgb(look.secondary))
                .child(key.clone()),
        );
    }
    title_line = title_line.child(
        div()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .font_weight(FontWeight::SEMIBOLD)
            .text_size(px(13.))
            .text_color(rgb(look.text))
            .child(item.title.clone()),
    );
    if let Some(meta) = &item.context_meta {
        title_line = title_line.child(
            div()
                .flex_shrink_0()
                .text_size(px(12.))
                .text_color(rgb(look.secondary))
                .child(meta.clone()),
        );
    }

    let mut left = col().flex_1().min_w_0().gap(px(2.)).child(title_line);
    if let Some(detail) = &item.detail_line {
        left = left.child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(12.))
                .text_color(rgb(look.secondary))
                .child(detail.clone()),
        );
    }

    let icon_bg = match (item.group, dark) {
        (ReviewGroup::LocalReadyToPush, false) => 0xf3e8ff,
        (ReviewGroup::LocalReadyToPush, true) => 0x2c1d3d,
        (ReviewGroup::OnGithub, false) => 0xe8f1ff,
        (ReviewGroup::OnGithub, true) => 0x162844,
    };
    let icon_fg = match (item.group, dark) {
        (ReviewGroup::LocalReadyToPush, false) => 0x7c3aed,
        (ReviewGroup::LocalReadyToPush, true) => 0xc084fc,
        (ReviewGroup::OnGithub, false) => 0x2563eb,
        (ReviewGroup::OnGithub, true) => 0x60a5fa,
    };

    row()
        .id(SharedString::from(item.id.clone()))
        .role(Role::Button)
        .aria_label(item.title.clone())
        .w_full()
        .px(px(14.))
        .py(px(11.))
        .gap(px(12.))
        .items_center()
        .cursor_pointer()
        .when(show_divider, |el| el.border_t_1().border_color(rgb(look.line)))
        .hover(|el| el.bg(rgb(look.fill)))
        .child(
            row()
                .size(px(28.))
                .flex_shrink_0()
                .rounded(px(7.))
                .bg(rgb(icon_bg))
                .items_center()
                .justify_center()
                .child(chrome::icon(IconName::GitFork, icon_fg, 13.)),
        )
        .child(left)
        .child(
            div()
                .flex_shrink_0()
                .px(px(8.))
                .py(px(2.))
                .rounded_full()
                .bg(rgb(chip_bg))
                .text_color(rgb(chip_fg))
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .child(item.chip.label.clone()),
        )
        .children(item.age.as_ref().map(|age| {
            div()
                .flex_shrink_0()
                .text_size(px(11.5))
                .text_color(rgb(look.tertiary))
                .child(age.clone())
        }))
        .child(chrome::icon(IconName::ChevronRight, look.tertiary, 13.))
        .on_click(move |_, _, cx| match &action {
            ReviewRowAction::OpenSession(id) => {
                ui.send(AppAction::Navigate(Route::Session(id.clone())), cx);
            }
            ReviewRowAction::StartTodoReview(key) => {
                ui.send(AppAction::Todo(TodoAction::StartReview(key.clone())), cx);
            }
            ReviewRowAction::OpenPrUrl(url) => {
                ui.send(watch_fallback.clone(), cx);
                ui.send(AppAction::PrDialog(DialogAction::Paste(url.clone())), cx);
            }
        })
        .test_support()
        .into_any_element()
}

fn draw_group(
    ui: &Rc<Ui>,
    state: &AppState,
    title: &'static str,
    hint: Option<&'static str>,
    rows: &[ReviewRow],
    look: Look,
) -> Div {
    let mut header = row()
        .gap(px(6.))
        .items_center()
        .pb(px(6.))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(look.secondary))
                .child(format!("{title} · {}", rows.len())),
        );
    if let Some(hint) = hint {
        header = header.child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(look.tertiary))
                .child(hint),
        );
    }

    let card = col()
        .w_full()
        .rounded(px(10.))
        .border_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.content))
        .overflow_hidden()
        .children(
            rows.iter()
                .enumerate()
                .map(|(i, item)| draw_row(ui, state, item, i > 0, look)),
        );

    col().w_full().child(header).child(card)
}

/// Renders the desktop `Reviews` inbox page.
pub fn page(ui: &Rc<Ui>, state: &AppState, look: Look) -> impl IntoElement {
    let data = build_reviews_inbox(state);
    let waiting_agents = humata2_inbox::waiting(&state.workspace) + state.pending_cli_logins.len();
    let watch_action = open_watch_pr_action(state);

    let refresh_ui = ui.clone();
    let watch_ui = ui.clone();
    let header = row()
        .id("reviews-inbox-header")
        .w_full()
        .px(px(28.))
        .py(px(14.))
        .gap(px(10.))
        .items_center()
        .border_b_1()
        .border_color(rgb(look.line))
        .child(
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(look.text))
                .child("Reviews"),
        )
        .child(
            div()
                .text_size(px(12.5))
                .text_color(rgb(look.secondary))
                .child(format!("{} waiting", data.total)),
        )
        .child(div().flex_1())
        .child(
            row()
                .gap(px(6.))
                .items_center()
                .text_size(px(12.))
                .text_color(rgb(look.tertiary))
                .child("GitHub checked ·")
                .child(
                    div()
                        .id("reviews-refresh")
                        .role(Role::Button)
                        .aria_label("Refresh reviews")
                        .cursor_pointer()
                        .text_color(rgb(look.accent))
                        .child("Refresh")
                        .on_click(move |_, _, cx| {
                            refresh_ui.send(AppAction::Todo(TodoAction::Load), cx);
                        })
                        .test_support(),
                ),
        )
        .child(
            Button::new("reviews-watch-pr")
                .outline()
                .small()
                .h(px(26.))
                .px(px(10.))
                .child("+ Watch PR URL…")
                .on_click(move |_, _, cx| {
                    watch_ui.send(watch_action.clone(), cx);
                }),
        );

    let body = if data.total == 0 {
        let empty_watch_ui = ui.clone();
        let empty_watch_action = open_watch_pr_action(state);
        col()
            .id("reviews-inbox-empty")
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .pb(px(64.))
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(look.text))
                    .child("Nothing waiting for review"),
            )
            .child(
                div()
                    .max_w(px(460.))
                    .text_center()
                    .text_size(px(12.5))
                    .text_color(rgb(look.secondary))
                    .child("Local stacks ready to push, GitHub feedback, review requests, and watched PRs appear here."),
            )
            .child(
                div().pt(px(6.)).child(
                    Button::new("reviews-empty-watch-pr")
                        .outline()
                        .small()
                        .h(px(28.))
                        .px(px(12.))
                        .child("+ Watch PR URL…")
                        .on_click(move |_, _, cx| {
                            empty_watch_ui.send(empty_watch_action.clone(), cx);
                        }),
                ),
            )
            .into_any_element()
    } else {
        let mut content = col()
            .id("reviews-inbox-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(28.))
            .py(px(20.))
            .gap(px(20.));

        if !data.local.is_empty() {
            content = content.child(draw_group(
                ui,
                state,
                "Local · ready to push",
                Some("Nothing leaves this machine until you approve"),
                &data.local,
                look,
            ));
        }
        if !data.github.is_empty() {
            content = content.child(draw_group(
                ui,
                state,
                "On GitHub · feedback & review requests",
                None,
                &data.github,
                look,
            ));
        }
        if waiting_agents > 0 {
            let inbox_ui = ui.clone();
            let agent_word = if waiting_agents == 1 {
                "1 agent is"
            } else {
                "agents are"
            };
            let banner_text = if waiting_agents == 1 {
                format!("{agent_word} stopped waiting for your answer")
            } else {
                format!("{waiting_agents} {agent_word} stopped waiting for your answer")
            };
            content = content.child(
                row()
                    .id("reviews-agent-inbox-banner")
                    .w_full()
                    .px(px(14.))
                    .py(px(10.))
                    .rounded(px(9.))
                    .border_1()
                    .border_color(rgb(look.line))
                    .bg(rgb(look.fill))
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.))
                            .text_color(rgb(look.secondary))
                            .child(banner_text),
                    )
                    .child(
                        div()
                            .id("reviews-open-agent-inbox")
                            .role(Role::Link)
                            .aria_label("Open Agent inbox")
                            .cursor_pointer()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(look.accent))
                            .child("Open Agent inbox →")
                            .on_click(move |_, _, cx| {
                                inbox_ui.send(AppAction::Switch(Route::Inbox), cx);
                            })
                            .test_support(),
                    ),
            );
        }

        content.into_any_element()
    };

    col()
        .id("reviews-inbox-page")
        .size_full()
        .bg(rgb(look.content))
        .child(header)
        .child(body)
        .test_support()
}

#[cfg(test)]
mod tests {
    use ::core::prelude::v1::test;

    use super::{
        concise_chip_label, format_layer_summary, parse_watch_pr_url, strip_row_noise,
    };

    #[test]
    fn parse_watch_pr_url_handles_urls_and_shorthand() {
        assert_eq!(
            parse_watch_pr_url("https://github.com/acme/web/pull/318"),
            Some(("acme".to_owned(), "web".to_owned(), 318))
        );
        assert_eq!(
            parse_watch_pr_url("https://github.com/acme/api/pull/42/files?w=1#diff-1"),
            Some(("acme".to_owned(), "api".to_owned(), 42))
        );
        assert_eq!(
            parse_watch_pr_url("acme/sdk#91"),
            Some(("acme".to_owned(), "sdk".to_owned(), 91))
        );
        assert_eq!(parse_watch_pr_url(""), None);
        assert_eq!(parse_watch_pr_url("https://github.com/acme/web/issues/12"), None);
        assert_eq!(parse_watch_pr_url("not-a-pr"), None);
    }

    #[test]
    fn format_layer_summary_omits_single_layer_noise() {
        assert_eq!(format_layer_summary(0, &[]), None);
        assert_eq!(format_layer_summary(1, &["only"]), None);
        assert_eq!(
            format_layer_summary(3, &["schema", "dual write", "reads"]),
            Some("3 layers · schema, dual write, reads".to_owned())
        );
        assert_eq!(
            format_layer_summary(4, &[]),
            Some("4 layers".to_owned())
        );
    }

    #[test]
    fn concise_chip_label_shortens_verbose_copy() {
        assert_eq!(concise_chip_label("Fixes pushed since you looked"), "Fixes pushed");
        assert_eq!(concise_chip_label("New feedback since you looked"), "New feedback");
        assert_eq!(concise_chip_label("Requested your review"), "Review requested");
        assert_eq!(concise_chip_label("Merge conflicts"), "Conflicts");
        assert_eq!(concise_chip_label(""), "New");
    }

    #[test]
    fn strip_row_noise_drops_boilerplate() {
        assert_eq!(strip_row_noise("no To-do, not your work"), None);
        assert_eq!(strip_row_noise("  No To-do, not your work  "), None);
        assert_eq!(strip_row_noise("   "), None);
        assert_eq!(
            strip_row_noise("dana requested changes · 2 inline comments"),
            Some("dana requested changes · 2 inline comments".to_owned())
        );
    }
}
