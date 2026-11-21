use super::*;
use humata2_clients::model::DiffBase;

pub(super) fn nothing_against(ui: &Rc<Ui>, state: &AppState, look: Look, base: &DiffBase) -> impl IntoElement {
    let (id, message) = match base {
        DiffBase::OriginMain => ("diff-no-changes-yet", "No changes against origin/main"),
        DiffBase::UncommittedOnly => ("diff-none-against-base", "Nothing uncommitted"),
        DiffBase::SinceLastReviewed => {
            ("diff-none-against-base", "No changes since you last reviewed")
        }
        DiffBase::SinceFork => ("diff-none-against-base", "No changes since the fork"),
        DiffBase::OtherSession(_) => ("diff-none-against-base", "No changes against that branch"),
        DiffBase::PullRequestBase => {
            ("diff-none-against-base", "No changes against the pull request's base")
        }
        DiffBase::Spec(_) => ("diff-none-against-base", "No changes against each repository's base"),
    };
    let message = stack_scope(state, base).map_or(message, StackScope::nothing);
    // DS-32: centred, with the one way out: pick another base.
    col()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .child(secondary(look, message).p(px(12.)).id(id).test_support())
        .child(action(
            ui,
            "diff-compare-with",
            "Compare with…",
            AppAction::Review(ReviewAction::ToggleBaseMenu),
            look,
            false,
        ))
}

/// The panel when the open session's first read failed: what failed, and
/// Try again. It stands in for the diff body (D179).
pub(super) fn read_failed_block(
    ui: &Rc<Ui>,
    state: &AppState,
    documents: &Documents,
    error: String,
    look: Look,
) -> impl IntoElement {
    use gpui_kit::base::Disableable as _;
    let block = col()
        .id("diff-read-failed")
        .flex_shrink_0()
        .gap(px(8.))
        .p(px(12.));
    // Frame 98: a clone with no default branch. Humata finds it; the
    // repository's own base chip is the other way out.
    let missing = state.session.open.as_ref().and_then(|id| {
        let fetch = state.review.fetch.get(id)?;
        let repo = fetch.no_default.clone()?;
        Some((id.clone(), repo, fetch.finding, fetch.find_error.clone()))
    });
    let Some((session, repo, finding, find_error)) = missing else {
        return block
            .child(secondary(look, "Couldn't read the changes."))
            .child(secondary(look, error))
            .child(action(
                ui,
                "diff-retry",
                "Try again",
                AppAction::Review(ReviewAction::Retry),
                look,
                false,
            ))
            .test_support();
    };
    let name = repo.rsplit('/').next().unwrap_or(repo.as_str()).to_owned();
    let find = AppAction::Review(ReviewAction::FindDefaultBranch { repo: repo.clone() });
    let block = match find_error {
        Some(reason) => block
            .child(secondary(look, format!("Couldn't find {name}'s default branch.")))
            .child(secondary(look, reason))
            .child(action(ui, "diff-find-default", "Try again", find, look, false).disabled(finding)),
        None => block
            .child(secondary(look, "Couldn't read the changes."))
            .child(secondary(look, format!("{name} has no default branch to compare with.")))
            .child(
                action(
                    ui,
                    "diff-find-default",
                    if finding { "Finding the default branch…" } else { "Find the default branch" },
                    find,
                    look,
                    true,
                )
                .disabled(finding),
            ),
    };
    let pick = RepoDiff {
        session,
        repo,
        // No default branch to name, so the chip asks for a pick (frame 98).
        base: "pick a branch…".to_owned(),
        files: Vec::new(),
        comments: Vec::new(),
    };
    block
        .child(
            // The hint and the chip stack, so the whole chip fits the panel
            // (frame 98).
            col()
                .gap(px(6.))
                .items_start()
                .child(secondary(look, "Or pick a branch to compare against."))
                .child(repo_base(ui, state, look, documents, usize::MAX, &pick)),
        )
        .test_support()
}

/// A repository whose read failed while the others were read (DS-35): its
/// name, why it failed, and Try again, which reads the session again.
pub(super) fn repo_error_row(
    ui: &Rc<Ui>,
    look: Look,
    repo: &str,
    message: &str,
    all_repos: &[&str],
) -> AnyElement {
    let retry = ui.clone();
    let shown_repo = display_repo_name(repo, all_repos.iter().copied());
    col()
        .id(SharedString::from(format!("repo-error-{repo}")))
        .gap(px(2.))
        .px(px(4.))
        .pb(px(4.))
        .child(
            row()
                .gap(px(6.))
                .h(px(26.))
                .child(icon(IconName::FolderGit2, look.secondary, 12.))
                .child(full_text(
                    text(shown_repo.to_owned())
                        .id(SharedString::from(format!("repo-error-name-{repo}")))
                        .font_weight(FontWeight::SEMIBOLD)
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis(),
                    repo_label(repo).to_owned(),
                ))
                .child(div().flex_1())
                .child(Quiet {
                    button: Button::new(SharedString::from(format!("repo-retry-{repo}")))
                        .small()
                        .h(px(18.))
                        .text_size(px(11.5))
                        .label("Try again")
                        .accessibility_label("Try again")
                        .flex_shrink_0()
                        .on_click(move |_, _, cx| {
                            retry.send(AppAction::Review(ReviewAction::Retry), cx)
                        }),
                    color: look.secondary,
                    fill: look.fill,
                }),
        )
        .child(secondary(look, message.to_owned()).pl(px(18.)))
        .test_support()
        .into_any_element()
}

/// A refresh that failed while the last diff read still shows below it
/// (D179): what failed, and Try again.
pub(super) fn refresh_failed_row(ui: &Rc<Ui>, error: String, look: Look) -> impl IntoElement {
    col()
        .id("diff-refresh-failed")
        .flex_shrink_0()
        .gap(px(2.))
        .px(px(12.))
        .py(px(6.))
        .border_b_1()
        .border_color(rgb(look.line))
        .child(
            row()
                .gap(px(8.))
                .child(secondary(
                    look,
                    "Couldn't refresh. Showing the last diff read.",
                ))
                .child(div().flex_1())
                .child(action(
                    ui,
                    "diff-retry",
                    "Try again",
                    AppAction::Review(ReviewAction::Retry),
                    look,
                    false,
                )),
        )
        .child(secondary(look, error))
        .test_support()
}
