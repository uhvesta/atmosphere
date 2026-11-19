use super::*;
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
        .h(px(26.))
        .px(px(4.))
        .pr(px(4.))
        .child(icon(IconName::FolderGit2, look.secondary, 12.))
        .child(text(repo_label(&diff.repo).to_owned()).font_weight(FontWeight::SEMIBOLD))
        .child(repo_base(ui, state, look, documents, index, diff))
        .child(div().flex_1())
}
