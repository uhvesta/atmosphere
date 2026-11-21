//! The Files panel: the clone's tree, filter and file actions.
use humata2_clients::model::{Agent, LineKind, SessionFile};

use crate::panel_prelude::*;

/// Virtual repo key for the session's current agent state files.
pub const AGENT_STATE_REPO: &str = "@agent-state";
const AGENT_STATE_PREV_PREFIX: &str = "@agent-state:prev:";
const AGENT_STATE_EXPANDED_KEY: &str = "__expanded__";

/// Whether `repo` is a virtual agent-state bucket rather than a normal repository.
#[must_use]
pub fn is_agent_state_repo(repo: &str) -> bool {
    repo == AGENT_STATE_REPO || repo.starts_with("@agent-state:")
}

/// Whether `agent` exposes readable agent state files in the Files panel.
/// Unsupported agents (Shell, Agy, OpenCode) hide the Agent files row completely.
#[must_use]
pub const fn supports_agent_files(agent: Agent) -> bool {
    matches!(agent, Agent::Claude | Agent::Codex | Agent::Copilot)
}

/// Whether the Agent files section is expanded in `folded_files`.
/// Absence of `("@agent-state", "__expanded__")` means collapsed by default.
#[must_use]
pub fn is_agent_files_expanded(folded_files: &std::collections::BTreeSet<(String, String)>) -> bool {
    folded_files.contains(&(
        AGENT_STATE_REPO.to_owned(),
        AGENT_STATE_EXPANDED_KEY.to_owned(),
    ))
}

/// Partition `session_files` into `(normal_files, agent_state_files)` so
/// `@agent-state*` entries never mix into the normal repository tree.
#[must_use]
pub fn partition_session_files(files: &[SessionFile]) -> (Vec<&SessionFile>, Vec<&SessionFile>) {
    let mut normal = Vec::new();
    let mut agent = Vec::new();
    for file in files {
        if is_agent_state_repo(&file.repo) {
            agent.push(file);
        } else {
            normal.push(file);
        }
    }
    (normal, agent)
}

/// A human-readable entry in the Agent files section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentFileEntry {
    pub repo: String,
    pub path: String,
    pub label: String,
}

/// Grouped, human-readable view of a session's `@agent-state*` files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentFilesTree {
    pub top_level: Vec<AgentFileEntry>,
    pub subagents: Vec<AgentFileEntry>,
    pub tool_outputs: Vec<AgentFileEntry>,
    pub plans: Vec<AgentFileEntry>,
    pub todos: Vec<AgentFileEntry>,
    pub earlier: Vec<AgentFileEntry>,
}

impl AgentFilesTree {
    /// Total number of leaf files across all groups.
    #[must_use]
    pub fn total_files(&self) -> usize {
        self.top_level.len()
            + self.subagents.len()
            + self.tool_outputs.len()
            + self.plans.len()
            + self.todos.len()
            + self.earlier.len()
    }

    /// Collapsed summary count text (`empty`, `1 file`, `N files`).
    #[must_use]
    pub fn count_summary(&self) -> String {
        match self.total_files() {
            0 => "empty".to_owned(),
            1 => "1 file".to_owned(),
            n => format!("{n} files"),
        }
    }

    /// Whether any entry matches a lowercase `filter`.
    #[must_use]
    pub fn matches_filter(&self, filter: &str) -> bool {
        if filter.is_empty() {
            return true;
        }
        self.top_level
            .iter()
            .chain(&self.subagents)
            .chain(&self.tool_outputs)
            .chain(&self.plans)
            .chain(&self.todos)
            .chain(&self.earlier)
            .any(|entry| entry_matches_filter(entry, filter))
    }
}

fn entry_matches_filter(entry: &AgentFileEntry, filter: &str) -> bool {
    filter.is_empty()
        || entry.label.to_lowercase().contains(filter)
        || entry.path.to_lowercase().contains(filter)
}

fn is_root_transcript(path: &str) -> bool {
    !path.contains('/') && (path.ends_with(".jsonl") || path == "events.jsonl")
}

fn subagent_label(file_name: &str) -> String {
    let stem = file_name
        .strip_prefix("agent-")
        .unwrap_or(file_name)
        .strip_suffix(".jsonl")
        .unwrap_or(file_name);
    if stem.is_empty() {
        file_name.to_owned()
    } else {
        stem.to_owned()
    }
}

/// Group raw `@agent-state*` `SessionFile` entries into human-readable buckets.
#[must_use]
pub fn build_agent_files_tree(files: &[&SessionFile]) -> AgentFilesTree {
    let mut tree = AgentFilesTree::default();
    for file in files {
        if file.folder {
            continue;
        }
        let path = file.path.trim_start_matches('/');
        if path.is_empty() {
            continue;
        }
        let file_name = path.rsplit('/').next().unwrap_or(path);

        if let Some(prev_id) = file.repo.strip_prefix(AGENT_STATE_PREV_PREFIX) {
            let short_id: String = prev_id.chars().take(8).collect();
            let label = if is_root_transcript(path) {
                if short_id.is_empty() {
                    "Transcript".to_owned()
                } else {
                    format!("Transcript ({short_id})")
                }
            } else if short_id.is_empty() {
                file_name.to_owned()
            } else {
                format!("{short_id} · {file_name}")
            };
            tree.earlier.push(AgentFileEntry {
                repo: file.repo.clone(),
                path: file.path.clone(),
                label,
            });
            continue;
        }

        if path.starts_with("subagents/")
            || (file_name.starts_with("agent-") && file_name.ends_with(".jsonl"))
        {
            tree.subagents.push(AgentFileEntry {
                repo: file.repo.clone(),
                path: file.path.clone(),
                label: subagent_label(file_name),
            });
        } else if path.starts_with("tool-results/") || path.starts_with("tool-outputs/") {
            tree.tool_outputs.push(AgentFileEntry {
                repo: file.repo.clone(),
                path: file.path.clone(),
                label: file_name.to_owned(),
            });
        } else if path.starts_with("plans/") {
            tree.plans.push(AgentFileEntry {
                repo: file.repo.clone(),
                path: file.path.clone(),
                label: file_name.to_owned(),
            });
        } else if path.starts_with("todos/") {
            tree.todos.push(AgentFileEntry {
                repo: file.repo.clone(),
                path: file.path.clone(),
                label: file_name.to_owned(),
            });
        } else if is_root_transcript(path) {
            tree.top_level.push(AgentFileEntry {
                repo: file.repo.clone(),
                path: file.path.clone(),
                label: "Transcript (current)".to_owned(),
            });
        } else {
            tree.top_level.push(AgentFileEntry {
                repo: file.repo.clone(),
                path: file.path.clone(),
                label: file_name.to_owned(),
            });
        }
    }
    // Keep "Transcript (current)" first among top-level items.
    tree.top_level.sort_by(|a, b| {
        let a_tx = a.label == "Transcript (current)";
        let b_tx = b.label == "Transcript (current)";
        b_tx.cmp(&a_tx).then_with(|| a.label.cmp(&b.label))
    });
    tree
}

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
    let raw_files: &[SessionFile] = state
        .workspace
        .session_files
        .get(&session.id)
        .map_or(&[], Vec::as_slice);
    let (normal_files, agent_files) = partition_session_files(raw_files);
    for file in normal_files {
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
    if supports_agent_files(session.agent) {
        let agent_tree = build_agent_files_tree(&agent_files);
        let filter_matches = !filter.is_empty() && agent_tree.matches_filter(&filter);
        let expanded = state
            .session
            .view()
            .is_some_and(|view| is_agent_files_expanded(&view.folded_files))
            || filter_matches;
        if filter.is_empty() || filter_matches {
            rows.push(agent_files_section(
                ui,
                state,
                session.agent,
                &agent_tree,
                expanded,
                &filter,
                look,
            ));
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

fn agent_file_row(
    ui: &Rc<Ui>,
    state: &AppState,
    entry: &AgentFileEntry,
    indent_px: f32,
    look: Look,
) -> AnyElement {
    let selected = state
        .session
        .view()
        .and_then(|view| view.file.as_ref())
        .is_some_and(|(r, p)| r == &entry.repo && p == &entry.path);
    let click_ui = ui.clone();
    let repo = entry.repo.clone();
    let path = entry.path.clone();
    row()
        .id(SharedString::from(format!(
            "agent-file {}/{}",
            entry.repo, entry.path
        )))
        .role(Role::ListItem)
        .aria_label(entry.label.clone())
        .test_support()
        .h(px(23.))
        .pl(px(indent_px))
        .pr(px(8.))
        .gap(px(6.))
        .rounded(px(5.))
        .when(selected, |r| r.bg(rgb(look.fill)))
        .child(text(entry.label.clone()).flex_1())
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            click_ui.send(
                AppAction::Session(SessionAction::SelectFile {
                    repo: repo.clone(),
                    path: path.clone(),
                }),
                cx,
            )
        })
        .into_any_element()
}

fn agent_group_rows(
    ui: &Rc<Ui>,
    state: &AppState,
    group_key: &str,
    title_text: String,
    entries: &[AgentFileEntry],
    filter: &str,
    look: Look,
) -> Vec<AnyElement> {
    let matching: Vec<&AgentFileEntry> = entries
        .iter()
        .filter(|entry| entry_matches_filter(entry, filter))
        .collect();
    if matching.is_empty() {
        return Vec::new();
    }
    let fold_path = format!("__group__:{group_key}");
    let folded = state.session.view().is_some_and(|view| {
        view.folded_files
            .contains(&(AGENT_STATE_REPO.to_owned(), fold_path.clone()))
    });
    let open = !folded || !filter.is_empty();
    let click_ui = ui.clone();
    let toggle_path = fold_path.clone();
    let mut out = Vec::new();
    out.push(
        row()
            .id(SharedString::from(format!("agent-group {group_key}")))
            .role(Role::Button)
            .aria_label(title_text.clone())
            .test_support()
            .h(px(23.))
            .pl(px(14.))
            .pr(px(8.))
            .gap(px(6.))
            .rounded(px(5.))
            .child(
                text(if open { "▾" } else { "▸" })
                    .text_size(px(11.))
                    .text_color(rgb(look.secondary)),
            )
            .child(text(title_text).text_color(rgb(look.text)))
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                click_ui.send(
                    AppAction::Session(SessionAction::ToggleFolder {
                        repo: AGENT_STATE_REPO.to_owned(),
                        path: toggle_path.clone(),
                    }),
                    cx,
                )
            })
            .into_any_element(),
    );
    if open {
        for entry in matching {
            out.push(agent_file_row(ui, state, entry, 28., look));
        }
    }
    out
}

fn agent_files_section(
    ui: &Rc<Ui>,
    state: &AppState,
    agent: Agent,
    tree: &AgentFilesTree,
    expanded: bool,
    filter: &str,
    look: Look,
) -> AnyElement {
    let provider = agent.name();
    let subtitle = if expanded {
        format!("· {provider}")
    } else {
        format!("· {provider} · {}", tree.count_summary())
    };
    let click_ui = ui.clone();
    let header = row()
        .id(SharedString::from("agent-files-toggle"))
        .role(Role::Button)
        .aria_label(format!("Agent files {subtitle}"))
        .test_support()
        .w_full()
        .items_center()
        .justify_between()
        .px(px(8.))
        .py(px(5.))
        .rounded(px(6.))
        .bg(rgb(look.fill))
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            click_ui.send(
                AppAction::Session(SessionAction::ToggleFolder {
                    repo: AGENT_STATE_REPO.to_owned(),
                    path: AGENT_STATE_EXPANDED_KEY.to_owned(),
                }),
                cx,
            )
        })
        .child(
            row()
                .items_center()
                .gap(px(5.))
                .child(
                    text(if expanded { "▾" } else { "▸" })
                        .text_size(px(11.))
                        .text_color(rgb(look.secondary)),
                )
                .child(
                    text("Agent files")
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(look.text)),
                )
                .child(text(subtitle).text_color(rgb(look.secondary))),
        )
        .child(
            text("🔒 read-only")
                .text_size(px(11.))
                .text_color(rgb(look.secondary)),
        );

    let mut section = col()
        .mt(px(8.))
        .pt(px(8.))
        .border_t_1()
        .border_color(rgb(look.line))
        .gap(px(2.))
        .child(header);

    if expanded {
        if tree.total_files() == 0 {
            section = section.child(
                div()
                    .pl(px(20.))
                    .py(px(4.))
                    .child(text("No agent files yet").text_color(rgb(look.secondary))),
            );
        } else {
            for entry in &tree.top_level {
                if entry_matches_filter(entry, filter) {
                    section = section.child(agent_file_row(ui, state, entry, 14., look));
                }
            }
            section = section.children(agent_group_rows(
                ui,
                state,
                "subagents",
                format!("Subagents ({})", tree.subagents.len()),
                &tree.subagents,
                filter,
                look,
            ));
            section = section.children(agent_group_rows(
                ui,
                state,
                "tool-outputs",
                format!("Tool outputs ({})", tree.tool_outputs.len()),
                &tree.tool_outputs,
                filter,
                look,
            ));
            section = section.children(agent_group_rows(
                ui,
                state,
                "plans",
                format!("Plans ({})", tree.plans.len()),
                &tree.plans,
                filter,
                look,
            ));
            section = section.children(agent_group_rows(
                ui,
                state,
                "todos",
                format!("Todos ({})", tree.todos.len()),
                &tree.todos,
                filter,
                look,
            ));
            section = section.children(agent_group_rows(
                ui,
                state,
                "earlier",
                format!("Earlier before /clear · {}", tree.earlier.len()),
                &tree.earlier,
                filter,
                look,
            ));
        }
    }

    section.into_any_element()
}

#[cfg(test)]
mod tests {
    use ::core::prelude::v1::test;
    use std::collections::BTreeSet;

    use super::*;

    fn sf(repo: &str, path: &str) -> SessionFile {
        SessionFile {
            repo: repo.to_owned(),
            path: path.to_owned(),
            folder: false,
        }
    }

    #[test]
    fn supported_vs_unsupported_agents() {
        assert!(supports_agent_files(Agent::Claude));
        assert!(supports_agent_files(Agent::Codex));
        assert!(supports_agent_files(Agent::Copilot));

        assert!(!supports_agent_files(Agent::Shell));
        assert!(!supports_agent_files(Agent::Agy));
        assert!(!supports_agent_files(Agent::OpenCode));
    }

    #[test]
    fn collapsed_by_default_and_expands_on_key() {
        let mut folded = BTreeSet::new();
        assert!(!is_agent_files_expanded(&folded));

        folded.insert((
            AGENT_STATE_REPO.to_owned(),
            AGENT_STATE_EXPANDED_KEY.to_owned(),
        ));
        assert!(is_agent_files_expanded(&folded));
    }

    #[test]
    fn partition_keeps_agent_state_out_of_normal_repos() {
        let files = vec![
            sf("ledger", "src/main.rs"),
            sf("", "notes.md"),
            sf("@agent-state", "7c1e2a90.jsonl"),
            sf("@agent-state:current", "subagents/agent-abc.jsonl"),
            sf("@agent-state:prev:11112222", "11112222.jsonl"),
        ];
        let (normal, agent) = partition_session_files(&files);
        assert_eq!(normal.len(), 2);
        assert_eq!(agent.len(), 3);
        assert!(normal.iter().all(|f| !is_agent_state_repo(&f.repo)));
        assert!(agent.iter().all(|f| is_agent_state_repo(&f.repo)));
    }

    #[test]
    fn human_readable_labels_and_grouping() {
        let files = vec![
            sf("@agent-state", "7c1e2a90-dead-beef.jsonl"),
            sf("@agent-state", "subagents/agent-check-readers.jsonl"),
            sf("@agent-state", "tool-results/bash-001.txt"),
            sf("@agent-state", "tool-outputs/grep-002.txt"),
            sf("@agent-state", "plans/migration.md"),
            sf("@agent-state", "todos/items.json"),
            sf("@agent-state", "workspace.yaml"),
            sf("@agent-state:prev:99887766aabb", "99887766aabb.jsonl"),
        ];
        let refs: Vec<&SessionFile> = files.iter().collect();
        let tree = build_agent_files_tree(&refs);

        assert_eq!(tree.total_files(), 8);
        assert_eq!(tree.count_summary(), "8 files");

        assert_eq!(tree.top_level.len(), 2);
        assert_eq!(tree.top_level[0].label, "Transcript (current)");
        assert_eq!(tree.top_level[1].label, "workspace.yaml");

        assert_eq!(tree.subagents.len(), 1);
        assert_eq!(tree.subagents[0].label, "check-readers");

        assert_eq!(tree.tool_outputs.len(), 2);
        assert_eq!(tree.tool_outputs[0].label, "bash-001.txt");
        assert_eq!(tree.tool_outputs[1].label, "grep-002.txt");

        assert_eq!(tree.plans.len(), 1);
        assert_eq!(tree.plans[0].label, "migration.md");

        assert_eq!(tree.todos.len(), 1);
        assert_eq!(tree.todos[0].label, "items.json");

        assert_eq!(tree.earlier.len(), 1);
        assert_eq!(tree.earlier[0].label, "Transcript (99887766)");
    }

    #[test]
    fn empty_and_single_count_summaries() {
        let empty = build_agent_files_tree(&[]);
        assert_eq!(empty.total_files(), 0);
        assert_eq!(empty.count_summary(), "empty");

        let one = [sf("@agent-state", "events.jsonl")];
        let refs: Vec<&SessionFile> = one.iter().collect();
        let single = build_agent_files_tree(&refs);
        assert_eq!(single.total_files(), 1);
        assert_eq!(single.count_summary(), "1 file");
        assert_eq!(single.top_level[0].label, "Transcript (current)");
    }
}
