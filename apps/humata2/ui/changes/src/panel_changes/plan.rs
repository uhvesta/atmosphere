use super::*;
use humata2_review::stack::WholeEntry;
use humata2_review::{PlanRow, PlanRowNote};

/// A plan note as it reads: "L12–14 · text", or "L12 · text" on one line, or
/// just the text for a note on the whole file.
fn plan_note_label(note: &PlanRowNote) -> String {
    match note.lines {
        Some((first, last)) if first == last => format!("L{first} · {}", note.text),
        Some((first, last)) => format!("L{first}–{last} · {}", note.text),
        None => note.text.clone(),
    }
}

/// A plan's rows (RV-32): a header and note for each group, then its files in
/// the plan's order with the agent's notes under them.
#[allow(clippy::too_many_arguments)]
pub(super) fn plan_list(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    documents: &Documents,
    diffs: &[&RepoDiff],
    picked: Option<(&RepoDiff, &humata2_clients::model::FileDiff)>,
    cursor: Option<&humata2_review::ListCursor>,
    plan: humata2_review::PlanLayout,
) -> Vec<AnyElement> {
    let mut groups = 0_usize;
    let mut outdated = 0_usize;
    // D867: Whole stack marks where each layer starts in each repository.
    let whole = state
        .session
        .open
        .as_ref()
        .and_then(|session| state.review.whole_groups(session, &plan, diffs));
    let humata2_review::PlanLayout { rows: plan, skims } = plan;
    let send = &crate::panel_stack::sender(ui);
    // DS-67: one folded row per repository's skim files, which open in
    // place under it.
    let draw_skim = |skim: &humata2_review::PlanSkim| -> Vec<AnyElement> {
        let open = crate::panel_stack::skim_open(&state.review, &skim.repo);
        let paths: Vec<&str> = skim.paths.iter().map(String::as_str).collect();
        let mut drawn: Vec<AnyElement> =
            crate::panel_stack::skim_row(look, &skim.repo, &paths, open, send).into_iter().collect();
        if open && let Some(diff) = diffs.iter().find(|diff| diff.repo == skim.repo) {
            drawn.extend(
                skim.paths
                    .iter()
                    .filter_map(|path| diff.files.iter().find(|file| &file.path == path))
                    .map(|file| {
                        let on = picked.is_some_and(|(d, f)| d.repo == diff.repo && f.path == file.path);
                        ringed(
                            file_row(ui, state, look, diff, file, on, false),
                            at_file(cursor, &diff.repo, &file.path),
                            look,
                        )
                    }),
            );
        }
        drawn
    };
    // Frames 119 to 121: a stack without groups reads repository by
    // repository, each head over its files and its skim row.
    let stacked = crate::panel_stack::header_line(&state.review).is_some()
        && !plan.iter().any(|entry| matches!(entry, PlanRow::Group { .. }));
    if stacked {
        let mut out = Vec::with_capacity(diffs.len() + plan.len());
        for (index, diff) in diffs.iter().enumerate() {
            let folded = state.session.open.as_ref().is_some_and(|id| {
                state.review.collapsed.contains(&(id.clone(), diff.repo.clone()))
            });
            out.push(ringed(
                stack_repo_head(ui, state, look, index, diff, folded, diffs).into_any_element(),
                at_repo(cursor, &diff.repo),
                look,
            ));
            if folded {
                continue;
            }
            if let Some(entries) = whole.as_ref().and_then(|groups| groups.get(index)) {
                for entry in entries {
                    match entry {
                        WholeEntry::Caption { number, title } => out.push(
                            crate::panel_stack::layer_caption(look, &diff.repo, Some((*number, title)), send),
                        ),
                        WholeEntry::Other => {
                            out.push(crate::panel_stack::layer_caption(look, &diff.repo, None, send));
                        }
                        WholeEntry::File { path, also } => {
                            let Some(file) = diff.files.iter().find(|file| &file.path == path) else {
                                continue;
                            };
                            let on = picked.is_some_and(|(d, f)| d.repo == diff.repo && f.path == *path);
                            let note = if also.is_empty() {
                                String::new()
                            } else {
                                let numbers: Vec<String> = also.iter().map(ToString::to_string).collect();
                                format!("also {}", numbers.join(", "))
                            };
                            out.push(ringed(
                                file_row_also(ui, state, look, diff, file, on, false, &note).into_any_element(),
                                at_file(cursor, &diff.repo, path),
                                look,
                            ));
                        }
                    }
                }
            } else {
                for entry in &plan {
                    let PlanRow::File { repo, path, .. } = entry else { continue };
                    if *repo != diff.repo {
                        continue;
                    }
                    let Some(file) = diff.files.iter().find(|file| &file.path == path) else {
                        continue;
                    };
                    let on = picked.is_some_and(|(d, f)| d.repo == *repo && f.path == *path);
                    out.push(ringed(
                        file_row(ui, state, look, diff, file, on, false),
                        at_file(cursor, repo, path),
                        look,
                    ));
                }
            }
            for skim in skims.iter().filter(|skim| skim.repo == diff.repo) {
                out.extend(draw_skim(skim));
            }
        }
        out.extend(crate::panel_stack::untouched_lines(&state.review, look));
        return out;
    }
    let mut skims = skims.into_iter().peekable();
    let mut out = Vec::with_capacity(diffs.len() + plan.len());
    out.extend(
        diffs
            .iter()
            .enumerate()
            .map(|(index, diff)| {
                ringed(
                    plan_repo_line(ui, state, look, documents, index, diff),
                    at_repo(cursor, &diff.repo),
                    look,
                )
            }),
    );
    for (at, entry) in plan.into_iter().enumerate() {
        while let Some(skim) = skims.next_if(|skim| skim.after == at) {
            out.extend(draw_skim(&skim));
        }
        match entry {
            PlanRow::Group { title, note } => {
                let n = groups;
                groups += 1;
                let mut header = col()
                    .px(px(4.))
                    .pt(px(8.))
                    .pb(px(2.))
                    .gap(px(2.))
                    .child(
                        text(title)
                            .id(SharedString::from(format!("plan-group-{n}")))
                            .test_support()
                            .font_weight(FontWeight::SEMIBOLD),
                    );
                if !note.is_empty() {
                    header = header.child(
                        secondary(look, note)
                            .id(SharedString::from(format!("plan-note-{n}")))
                            .test_support(),
                    );
                }
                out.push(header.into_any_element());
            }
            PlanRow::File { repo, path, notes } => {
                let Some(diff) = diffs.iter().find(|diff| diff.repo == repo) else {
                    continue;
                };
                let Some(file) = diff.files.iter().find(|file| file.path == path) else {
                    continue;
                };
                let on = picked.is_some_and(|(d, f)| d.repo == repo && f.path == path);
                let rows: Vec<Div> = notes
                    .into_iter()
                    .map(|note| {
                        let mut line = row()
                            .pl(px(40.))
                            .pr(px(8.))
                            .pb(px(2.))
                            .gap(px(6.))
                            .child(secondary(look, plan_note_label(&note)));
                        if note.outdated {
                            let n = outdated;
                            outdated += 1;
                            line = line.child(
                                secondary(look, "outdated")
                                    .id(SharedString::from(format!("plan-note-outdated-{n}")))
                                    .test_support(),
                            );
                        }
                        line
                    })
                    .collect();
                out.push(
                    col()
                        .child(ringed(
                            file_row(ui, state, look, diff, file, on, false),
                            at_file(cursor, &repo, &path),
                            look,
                        ))
                        .children(rows)
                        .into_any_element(),
                );
            }
        }
    }
    for skim in skims {
        out.extend(draw_skim(&skim));
    }
    out
}

/// A plan's line for one repository (RV-31 with RV-32): the plan's groups can
/// mix repositories, so each repository's base sits once above them.
fn plan_repo_line(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    documents: &Documents,
    index: usize,
    diff: &RepoDiff,
) -> Div {
    row()
        .gap(px(6.))
        .h(px(24.))
        .px(px(4.))
        .pr(px(4.))
        .min_w_0()
        .whitespace_nowrap()
        .child(div().flex_shrink_0().child(icon(IconName::FolderGit2, look.secondary, 12.)))
        .child(
            text(repo_label(&diff.repo).to_owned())
                .font_weight(FontWeight::SEMIBOLD)
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis(),
        )
        // D868: with one repository the header's chip names the base.
        .when(
            state.workspace.diffs.iter().filter(|each| each.session == diff.session).count() >= 2,
            |line| line.child(repo_base(ui, state, look, documents, index, diff)),
        )
        .child(div().flex_1())
}

/// A stack's line for one repository (frames 119 to 121): the fold chevron,
/// its short name, "against main" while Whole stack shows (a layer's base is
/// the layer under it), and its totals.
fn stack_repo_head(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    index: usize,
    diff: &RepoDiff,
    folded: bool,
    diffs: &[&RepoDiff],
) -> impl IntoElement {
    let (added, removed) = diff.files.iter().fold((0_u64, 0_u64), |(a, r), file| {
        (a + u64::from(file.added), r + u64::from(file.removed))
    });
    let whole = state
        .session
        .open
        .as_ref()
        .is_some_and(|id| state.review.layer_of(id).is_none());
    let toggle = ui.clone();
    let repo = diff.repo.clone();
    let name =
        crate::panel_stack::stack_repo_name(&diff.repo, diffs.iter().map(|each| each.repo.as_str())).to_owned();
    // A repository with nothing changed has nothing to fold or count.
    let empty = diff.files.is_empty();
    row()
        .id(SharedString::from(format!("repo-row-{index}")))
        .test_support()
        .gap(px(6.))
        .h(px(24.))
        .px(px(4.))
        .pr(px(8.))
        .min_w_0()
        .whitespace_nowrap()
        .when(empty, |line| line.child(div().w(px(20.)).flex_shrink_0()))
        .when(!empty, |line| line.child(
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
                .accessibility_label(format!("{} {}", if folded { "Expand" } else { "Collapse" }, diff.repo))
                .on_click(move |_, _, cx| {
                    toggle.send(AppAction::Review(ReviewAction::ToggleRepo { repo: repo.clone() }), cx)
                }),
        ))
        .child(
            text(name)
                .id(SharedString::from(format!("repo-name-{index}")))
                .test_support()
                .font_weight(FontWeight::SEMIBOLD)
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis(),
        )
        .when(whole && !diff.base.is_empty(), |line| {
            line.child(
                secondary(look, format!("against {}", diff.base))
                    .id(SharedString::from(format!("repo-base-{index}")))
                    .test_support()
                    .min_w_0()
                    .flex_shrink(8.)
                    .overflow_hidden()
                    .text_ellipsis(),
            )
        })
        .child(div().flex_1())
        .when(!empty, |line| line.child(
            row()
                .id(SharedString::from(format!("repo-totals-{index}")))
                .test_support()
                .gap(px(6.))
                .flex_shrink_0()
                .child(text(format!("+{added}")).text_size(px(11.)).text_color(rgb(GREEN)))
                .child(text(format!("\u{2212}{removed}")).text_size(px(11.)).text_color(rgb(RED))),
        ))
}
