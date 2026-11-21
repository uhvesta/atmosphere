//! The Files panel: the clone's tree, filter and file actions.
use humata2_clients::model::LineKind;

use crate::panel_prelude::*;

#[derive(Default)]
pub(super) struct FileNode {
    folder: bool,
    /// When it first arrived: folders keep the order they were reported in.
    order: usize,
    children: std::collections::BTreeMap<String, FileNode>,
}

pub fn file_rows(
    ui: &Rc<Ui>,
    state: &AppState,
    repo: &str,
    node: &FileNode,
    path: &str,
    depth: usize,
    filter: &str,
    look: Look,
) -> Vec<AnyElement> {
    let mut children: Vec<_> = node.children.iter().collect();
    children.sort_by(|(an, a), (bn, b)| {
        b.folder.cmp(&a.folder).then_with(|| {
            if a.folder {
                a.order.cmp(&b.order).then_with(|| an.cmp(bn))
            } else {
                an.cmp(bn)
            }
        })
    });
    let mut rows = Vec::new();
    for (name, child) in children {
        let path = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}/{name}")
        };
        if !filter.is_empty()
            && !path.to_lowercase().contains(filter)
            && !contains_file(child, &path, filter)
        {
            continue;
        }
        let folded = state
            .session
            .view()
            .is_some_and(|view| view.folded_files.contains(&(repo.to_owned(), path.clone())));
        let selected = state
            .session
            .view()
            .and_then(|view| view.file.as_ref())
            .is_some_and(|(r, p)| r == repo && p == &path);
        let changed = state
            .workspace
            .diffs
            .iter()
            .filter(|diff| Some(&diff.session) == state.session.open.as_ref() && diff.repo == repo)
            .flat_map(|diff| &diff.files)
            .find(|file| file.path == path)
            // A file whose every line is added is new; anything else changed.
            .map(|file| {
                file.removed == 0
                    && file
                        .lines
                        .iter()
                        .all(|line| matches!(line.kind, LineKind::Added | LineKind::Hunk))
            });
        let event = if child.folder {
            SessionAction::ToggleFolder {
                repo: repo.to_owned(),
                path: path.clone(),
            }
        } else {
            SessionAction::SelectFile {
                repo: repo.to_owned(),
                path: path.clone(),
            }
        };
        let label = if repo.is_empty() {
            path.clone()
        } else {
            format!("{repo}/{path}")
        };
        let click_ui = ui.clone();
        rows.push(
            row()
                .id(SharedString::from(format!(
                    "session-{} {repo}/{path}",
                    if child.folder { "folder" } else { "file" }
                )))
                .role(if child.folder {
                    Role::Button
                } else {
                    Role::ListItem
                })
                .aria_label(label)
                .test_support()
                .h(px(23.))
                .pl(px(8. + depth as f32 * 14.))
                .pr(px(8.))
                .gap(px(6.))
                .rounded(px(5.))
                .when(selected, |row| row.bg(rgb(look.fill)))
                .child(icon(
                    if child.folder {
                        if folded || child.children.is_empty() {
                            IconName::Folder
                        } else {
                            IconName::FolderOpen
                        }
                    } else if is_code(name) {
                        IconName::FileCode
                    } else {
                        IconName::FileText
                    },
                    look.secondary,
                    12.,
                ))
                .child(text(name.clone()).flex_1())
                .when_some(changed.filter(|_| !child.folder), |row, added| {
                    row.child(
                        text(if added { "A" } else { "M" })
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(if added { GREEN } else { ORANGE })),
                    )
                })
                .cursor_pointer()
                .on_click(move |_, _, cx| click_ui.send(AppAction::Session(event.clone()), cx))
                .into_any_element(),
        );
        if child.folder && (!folded || !filter.is_empty()) {
            rows.extend(file_rows(
                ui,
                state,
                repo,
                child,
                &path,
                depth + 1,
                filter,
                look,
            ));
        }
    }
    rows
}

/// Whether a file name is source code, as opposed to prose or configuration.
pub fn is_code(name: &str) -> bool {
    let extension = name.rsplit_once('.').map_or("", |(_, extension)| extension);
    !matches!(
        extension.to_ascii_lowercase().as_str(),
        "md" | "toml" | "txt" | "json" | "yaml" | "yml" | "lock" | "cfg" | "ini" | ""
    )
}

pub fn contains_file(node: &FileNode, path: &str, filter: &str) -> bool {
    node.children.iter().any(|(name, child)| {
        let path = format!("{path}/{name}");
        path.to_lowercase().contains(filter) || contains_file(child, &path, filter)
    })
}

pub fn files(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    full: bool,
    input: Entity<InputState>,
    look: Look,
) -> Div {
    // DK-3: a Desk session's panel lists its folder, with its clones and
    // the note about repos (frame 38).
    if let Some(panel) = humata2_ui_desk::desk_files::panel(ui, state, session, look) {
        return col().size_full().child(panel);
    }
    // Session-root files (`repo == ""`) sit outside clones; the session's own
    // repository comes first among clones, then the rest in arrival order.
    let mut session_root = FileNode::default();
    let mut repos = Vec::<(String, FileNode)>::new();
    let mut arrived = 0_usize;
    if !session.repo.is_empty() {
        repos.push((session.repo.clone(), FileNode::default()));
    }
    for file in state
        .workspace
        .session_files
        .get(&session.id)
        .into_iter()
        .flatten()
    {
        let mut node = if file.repo.is_empty() {
            &mut session_root
        } else {
            let at = repos
                .iter()
                .position(|(repo, _)| *repo == file.repo)
                .unwrap_or_else(|| {
                    repos.push((file.repo.clone(), FileNode::default()));
                    repos.len() - 1
                });
            &mut repos[at].1
        };
        for component in file.path.split('/').filter(|part| !part.is_empty()) {
            node.folder = true;
            arrived += 1;
            node = node
                .children
                .entry(component.to_owned())
                .or_insert_with(|| FileNode {
                    order: arrived,
                    ..FileNode::default()
                });
        }
        node.folder |= file.folder;
    }
    let filter = state
        .session
        .view()
        .map_or("", |view| view.file_filter.as_str())
        .to_lowercase();
    let mut rows = Vec::new();
    if !session_root.children.is_empty() {
        let children = file_rows(ui, state, "", &session_root, "", 1, &filter, look);
        if filter.is_empty() || !children.is_empty() {
            let folded = state
                .session
                .view()
                .is_some_and(|view| view.folded_files.contains(&(String::new(), String::new())));
            let click_ui = ui.clone();
            rows.push(
                row()
                    .id(SharedString::from("session-folder /"))
                    .role(Role::Button)
                    .aria_label("Session folder")
                    .test_support()
                    .h(px(23.))
                    .px(px(8.))
                    .gap(px(6.))
                    .child(icon(
                        if folded {
                            IconName::Folder
                        } else {
                            IconName::FolderOpen
                        },
                        look.secondary,
                        12.,
                    ))
                    .child(text("Session folder"))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        click_ui.send(
                            AppAction::Session(SessionAction::ToggleFolder {
                                repo: String::new(),
                                path: String::new(),
                            }),
                            cx,
                        )
                    })
                    .into_any_element(),
            );
            if !folded || !filter.is_empty() {
                rows.extend(children);
            }
        }
    }
    for (repo, node) in &repos {
        if node.children.is_empty() && repo != &session.repo {
            continue;
        }
        let children = file_rows(ui, state, repo, node, "", 1, &filter, look);
        if !filter.is_empty() && children.is_empty() {
            continue;
        }
        let folded = state
            .session
            .view()
            .is_some_and(|view| view.folded_files.contains(&(repo.clone(), String::new())));
        let repo_name = repo.rsplit('/').next().unwrap_or(repo);
        let click_ui = ui.clone();
        let repo_id = repo.clone();
        rows.push(
            row()
                .id(SharedString::from(format!("session-folder {repo}/")))
                .role(Role::Button)
                .aria_label(repo.clone())
                .test_support()
                .h(px(23.))
                .px(px(8.))
                .gap(px(6.))
                .child(icon(IconName::FolderGit2, look.secondary, 12.))
                .child(text(repo_name.to_owned()))
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    click_ui.send(
                        AppAction::Session(SessionAction::ToggleFolder {
                            repo: repo_id.clone(),
                            path: String::new(),
                        }),
                        cx,
                    )
                })
                .into_any_element(),
        );
        if !folded || !filter.is_empty() {
            rows.extend(children);
        }
    }
    // REPOS (CH-21): Add repo to this session, in the header so the footer's
    // buttons keep the panel's padding. A clone can't be made on a machine
    // that is offline, so it waits like Open in editor and Upload… do.
    let offline = matches!(session.status, Status::Offline);
    let add_repo = row().child(
        action(
            ui,
            "files-add-repo",
            "Add repo…",
            AppAction::SessionAddRepo(humata2_app::SessionAddRepoAction::Open(session.id.clone())),
            look,
            false,
        )
        .disabled(offline),
    );
    col()
        .size_full()
        .child(panel_header(ui, state, Panel::Files, full, Some(add_repo), look))
        .child(
            div()
                .id("session-file-filter")
                .test_support()
                .p(px(10.))
                .child(
                    Input::new(&input)
                        .aria_label("Filter files")
                        .small()
                        // `h` only applies to multi-line inputs in the kit.
                        .min_h(px(26.))
                        .bg(rgb(look.card))
                        .border_color(rgb(look.line))
                        .text_color(rgb(look.text)),
                ),
        )
        .child(
            col()
                .id("session-files")
                .role(Role::List)
                .aria_label("Session files")
                .test_support()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(6.))
                .text_size(px(12.5))
                .children(rows),
        )
        .child(
            row()
                .gap(px(8.))
                .p(px(10.))
                .border_t_1()
                .border_color(rgb(look.line))
                .child(
                    action(
                        ui,
                        "files-open-big",
                        "Open big",
                        AppAction::Session(SessionAction::OpenFile),
                        look,
                        false,
                    )
                    .disabled(state.session.view().is_none_or(|v| v.file.is_none())),
                )
                .child(
                    Button::new("files-open-editor")
                        .label("Open in editor")
                        .small()
                        .text_color(rgb(look.text))
                        .opacity(1.)
                        .disabled(true),
                )
                .child(
                    Button::new("files-upload")
                        .label("Upload…")
                        .small()
                        .text_color(rgb(look.text))
                        .opacity(1.)
                        .disabled(true),
                )
        )
}
