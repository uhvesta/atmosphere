//! What an agent's Inbox card tells the owner besides the action itself
//! (INBOXHOOKS): the From/To/Carries rows, what else the action affects,
//! whether it can be undone, which board item the agent is on, and the rule
//! that asked. All of it is worked out here from what the card already
//! holds, in names rather than ids.

use std::fmt::Write as _;

use humata2_access::{Capability, Entry, Place, Root};
use humata2_daemon_ports::Names;
use humata2_grpc::humata::v1 as pb;
use humata2_store::{BoardItemRecord, Store};

/// The rows a card shows under its title.
#[derive(Debug, Default)]
pub(super) struct Details {
    pub facts: Vec<pb::CardFact>,
    pub affects: Vec<pb::CardEffect>,
    pub undo_text: String,
}

/// The rows for a card whose preview is `preview`; none for a preview that
/// has nothing more to say than it shows.
pub(super) fn details(preview: &pb::request_preview::Preview, names: &Names) -> Details {
    match preview {
        pb::request_preview::Preview::CrossMachineCopy(copy) => copy_details(copy, names),
        pb::request_preview::Preview::Move(preview) => move_details(preview, names),
        pb::request_preview::Preview::FileAccess(access) => file_access_details(access, names),
        _ => Details::default(),
    }
}

/// The rows for a file-read ask whose `FileAccessPreview` and policy `Place`
/// are both known, resolving session titles, agents and channel names from
/// `store` so the card shows names instead of ids (IN-10, INBOXHOOKS).
pub(super) fn file_read_details(
    access: &pb::FileAccessPreview,
    place: &Place,
    store: &Store,
    names: &Names,
) -> Details {
    let machine = machine_name(&access.source_machine, names);
    let mut session_title: Option<String> = None;
    let mut session_agent: Option<String> = None;
    let from = match &place.root {
        Root::Clone(session) | Root::Pastes(session) => {
            let record = store.session(session.id.as_str()).ok().flatten();
            let title = record
                .as_ref()
                .map(|r| r.title.clone())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| session.id.to_string());
            let agent = record
                .as_ref()
                .map(|r| r.agent.clone())
                .filter(|a| !a.is_empty());
            session_title = Some(title.clone());
            session_agent.clone_from(&agent);
            let mut rest = String::new();
            if let Some(agent) = &agent {
                let _ = write!(rest, " ({agent}'s session)");
            }
            if matches!(place.root, Root::Pastes(_)) {
                rest.push_str(" · pasted files");
            }
            if !machine.is_empty() {
                let _ = write!(rest, " · {machine}");
            }
            fact("From", title, rest)
        }
        Root::Shared(channel) => {
            let name = channel_name(store, channel.as_str());
            fact("From", machine.clone(), format!(" · {name} · shared"))
        }
        Root::ChannelFolder(channel) => {
            let name = channel_name(store, channel.as_str());
            fact("From", machine.clone(), format!(" · {name}"))
        }
        Root::MainClone { channel, repo } => {
            let name = channel_name(store, channel.as_str());
            fact("From", machine.clone(), format!(" · {name} · main clone ({repo})"))
        }
        Root::Home | Root::Received | Root::Outside => fact("From", machine.clone(), String::new()),
    };
    let files_text = format_paths(&access.paths);
    let files_row = if files_text.is_empty() {
        fact("Files", "The whole folder".to_owned(), String::new())
    } else if place.entry == Entry::Folder {
        fact("Files", files_text, " · folder".to_owned())
    } else {
        fact("Files", files_text, String::new())
    };
    let (access_row, undo_text) = access_and_undo(access.mode);
    let mut affects = Vec::new();
    match (session_title, session_agent) {
        (Some(title), Some(agent)) => affects.push(pb::CardEffect {
            warn: false,
            lead: format!("Nothing changes in {title}"),
            rest: format!("; {agent} isn't interrupted"),
        }),
        (Some(title), None) => affects.push(pb::CardEffect {
            warn: false,
            lead: format!("Nothing changes in {title}"),
            rest: "; this is a read, not a copy or edit".to_owned(),
        }),
        (None, _) => affects.push(pb::CardEffect {
            warn: false,
            lead: format!("Nothing changes on {machine}"),
            rest: "; this is a read, not a copy or edit".to_owned(),
        }),
    }
    if place.entry == Entry::Folder {
        affects.push(pb::CardEffect {
            warn: false,
            lead: "Lists names and sizes only".to_owned(),
            rest: "; file contents are not read".to_owned(),
        });
    } else {
        affects.push(pb::CardEffect {
            warn: false,
            lead: "Reads only what is named above".to_owned(),
            rest: "; other files stay unreadable".to_owned(),
        });
    }
    Details {
        facts: vec![from, files_row, access_row],
        affects,
        undo_text,
    }
}

fn channel_name(store: &Store, id: &str) -> String {
    store
        .channel(id)
        .ok()
        .flatten()
        .map(|channel| channel.name)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| id.to_owned())
}

fn format_paths(paths: &[pb::RepoPath]) -> String {
    paths
        .iter()
        .map(|entry| {
            let repo = entry
                .repo
                .as_ref()
                .filter(|r| !r.owner.is_empty() && !r.name.is_empty())
                .map(|r| format!("{}/{}", r.owner, r.name))
                .unwrap_or_default();
            match (repo.is_empty(), entry.path.is_empty()) {
                (_, true) => repo,
                (true, false) => entry.path.clone(),
                (false, false) => format!("{repo}/{}", entry.path),
            }
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

fn access_and_undo(mode: i32) -> (pb::CardFact, String) {
    match pb::file_access_preview::Mode::try_from(mode) {
        Ok(pb::file_access_preview::Mode::CopyIntoClone) => (
            fact("Access", "Copy into clone".to_owned(), String::new()),
            "By hand: delete the copied files. The originals are untouched.".to_owned(),
        ),
        Ok(pb::file_access_preview::Mode::CopyIntoShared) => (
            fact("Access", "Copy into shared".to_owned(), String::new()),
            "By hand: delete the copied files. The originals are untouched.".to_owned(),
        ),
        _ => (
            fact("Access", "Read only".to_owned(), " · for this call".to_owned()),
            "Nothing changes; there is nothing to undo.".to_owned(),
        ),
    }
}

fn file_access_details(access: &pb::FileAccessPreview, names: &Names) -> Details {
    let machine = machine_name(&access.source_machine, names);
    let session = access
        .source_session
        .as_ref()
        .map(|id| id.value.as_str())
        .filter(|id| !id.is_empty());
    let from = match session {
        Some(session) if !machine.is_empty() => {
            fact("From", session.to_owned(), format!(" · {machine}"))
        }
        Some(session) => fact("From", session.to_owned(), String::new()),
        None => fact("From", machine.clone(), String::new()),
    };
    let files_text = format_paths(&access.paths);
    let files_row = fact(
        "Files",
        if files_text.is_empty() {
            "The whole folder".to_owned()
        } else {
            files_text
        },
        String::new(),
    );
    let (access_row, undo_text) = access_and_undo(access.mode);
    let affects = vec![pb::CardEffect {
        warn: false,
        lead: match session {
            Some(session) => format!("Nothing changes in {session}"),
            None => format!("Nothing changes on {machine}"),
        },
        rest: "; this is a read, not a copy or edit".to_owned(),
    }];
    Details {
        facts: vec![from, files_row, access_row],
        affects,
        undo_text,
    }
}

fn fact(label: &str, lead: String, rest: String) -> pb::CardFact {
    pb::CardFact {
        label: label.to_owned(),
        lead,
        rest,
    }
}

fn machine_name(machine: &Option<pb::MachineId>, names: &Names) -> String {
    machine
        .as_ref()
        .map(|machine| names.of(&machine.value))
        .unwrap_or_default()
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn copy_details(copy: &pb::CrossMachineCopyPreview, names: &Names) -> Details {
    let source = machine_name(&copy.source_machine, names);
    let destination = machine_name(&copy.destination_machine, names);
    let landing = if copy.destination_place.is_empty() {
        copy.destination_path.clone()
    } else {
        format!("{} · {}", copy.destination_place, copy.destination_path)
    };
    let size = format!(
        "{} file{} · {}",
        copy.files,
        if copy.files == 1 { "" } else { "s" },
        size_text(copy.bytes)
    );
    let mut affects = Vec::new();
    if !copy.replaced.is_empty() {
        let listed = copy
            .replaced
            .iter()
            .map(|file| {
                if file.note.is_empty() {
                    file.path.clone()
                } else {
                    format!("{} ({})", file.path, file.note)
                }
            })
            .collect::<Vec<_>>();
        affects.push(pb::CardEffect {
            warn: true,
            lead: format!(
                "{} on {destination} {} replaced:",
                plural(listed.len(), "file", "files"),
                if listed.len() == 1 { "is" } else { "are" }
            ),
            rest: format!(" {}", join_and(&listed)),
        });
    }
    if copy.moves {
        affects.push(pb::CardEffect {
            warn: true,
            lead: format!("The folder leaves {source}"),
            rest: ": this is a move, not a copy".to_owned(),
        });
    } else {
        affects.push(pb::CardEffect {
            warn: false,
            lead: format!("Nothing changes on {source}"),
            rest: "; the files are copied, not moved".to_owned(),
        });
    }
    if copy.files as usize > copy.replaced.len() && !copy.replaced.is_empty() {
        let new = copy.files as usize - copy.replaced.len();
        affects.push(pb::CardEffect {
            warn: false,
            lead: format!(
                "{} on {destination} {} new",
                plural(new, "file", "files"),
                if new == 1 { "is" } else { "are" }
            ),
            rest: String::new(),
        });
    }
    // What the transfers service does with replaced files: they go to the
    // trash on the destination machine, kept for 30 days.
    let replaced_note = if copy.replaced.is_empty() {
        String::new()
    } else {
        format!(
            " The {} replaced {} in the trash on {destination} for 30 days.",
            plural(copy.replaced.len(), "file", "files"),
            if copy.replaced.len() == 1 { "waits" } else { "wait" }
        )
    };
    let undo_text = if copy.moves {
        format!("By hand: move the folder back from {destination}.{replaced_note}")
    } else {
        format!("By hand: delete the copied files. The originals are untouched.{replaced_note}")
    };
    Details {
        facts: vec![
            fact("From", source.clone(), format!("· {}", copy.source_path.replace('\n', ", "))),
            fact("To", destination, format!("· {landing}")),
            fact("Carries", size, String::new()),
        ],
        affects,
        undo_text,
    }
}

/// "a", "a and b", "a, b and c".
fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Where a session is, in words: the machine, then its channel and title.
fn session_place(machine: String, preview: &pb::MovePreview) -> pb::CardFact {
    let mut rest = String::new();
    for part in [&preview.channel_name, &preview.session_title] {
        if !part.is_empty() {
            rest.push_str(&format!("· {part} "));
        }
    }
    fact("", machine, rest.trim_end().to_owned())
}

fn move_details(preview: &pb::MovePreview, names: &Names) -> Details {
    let from = machine_name(&preview.from, names);
    let to = machine_name(&preview.to, names);
    let mut facts = Vec::new();
    for (label, machine) in [("From", from.clone()), ("To", to.clone())] {
        let mut row = session_place(machine, preview);
        row.label = label.to_owned();
        facts.push(row);
    }
    let mut carries = Vec::new();
    if !preview.repos.is_empty() {
        carries.push(format!(
            "{} ({})",
            plural(preview.repos.len(), "repo", "repos"),
            preview.repos.join(", ")
        ));
    }
    if preview.bytes > 0 {
        carries.push(size_text(preview.bytes));
    }
    if preview.uncommitted_files > 0 {
        carries.push(format!(
            "{} uncommitted {}, as they are",
            preview.uncommitted_files,
            if preview.uncommitted_files == 1 { "file" } else { "files" }
        ));
    }
    if !carries.is_empty() {
        facts.push(fact("Carries", carries.join(" · "), String::new()));
    }
    let mut affects = Vec::new();
    if !preview.terminals.is_empty() {
        affects.push(pb::CardEffect {
            warn: false,
            lead: format!(
                "{} move along and keep running:",
                plural(preview.terminals.len(), "terminal", "terminals")
            ),
            rest: format!(" {}", join_and(&preview.terminals)),
        });
    }
    for repo in &preview.clones_needed {
        affects.push(pb::CardEffect {
            warn: true,
            lead: format!("{to} has no clone of {repo} yet"),
            rest: "; Humata clones it there first".to_owned(),
        });
    }
    for fork in &preview.forks_staying {
        affects.push(pb::CardEffect {
            warn: true,
            lead: format!("Its fork, {fork}, stays on {from}"),
            rest: String::new(),
        });
    }
    Details {
        facts,
        affects,
        undo_text: format!(
            "Yes. Move it back at any time; nothing is removed from {from} until the move has finished."
        ),
    }
}

fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// The rule's name as Settings > Agent access shows it: the capability's
/// description, capitalised.
pub(super) fn rule_label(capability: Capability) -> String {
    let label = capability.label();
    let mut chars = label.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// "Working on PAY-3 Ledger schema v2" when `session` is on one of `items`;
/// empty when it is on none.
pub(super) fn working_on(items: &[BoardItemRecord], session: &str) -> String {
    items
        .iter()
        .find(|item| !item.deleted && item.sessions.iter().any(|on| on == session))
        .map(|item| format!("Working on {} {}", item.key, item.title))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copy() -> pb::CrossMachineCopyPreview {
        pb::CrossMachineCopyPreview {
            source_machine: Some(pb::MachineId {
                value: "devbox-1".to_owned(),
            }),
            source_path: "api/src/retry/backoff.rs".to_owned(),
            destination_machine: Some(pb::MachineId {
                value: "host".to_owned(),
            }),
            destination_path: "shared/backoff.rs".to_owned(),
            bytes: 4096,
            files: 1,
            destination_place: "shared".to_owned(),
            ..Default::default()
        }
    }

    fn item(sessions: &[&str], deleted: bool) -> BoardItemRecord {
        BoardItemRecord {
            key: "PAY-3".to_owned(),
            board: humata2_store::BoardKey::Global,
            channel: String::new(),
            title: "Ledger schema v2".to_owned(),
            description: String::new(),
            column: 1,
            rank: String::new(),
            created_by: String::new(),
            created_unix_ms: 0,
            updated_unix_ms: 0,
            revision: 1,
            done_unix_ms: None,
            deleted,
            sessions: sessions.iter().map(|s| (*s).to_owned()).collect(),
            depends_on: Vec::new(),
        }
    }

    #[test]
    fn a_copy_names_both_machines_and_says_it_can_be_undone() {
        let names = Names::of_host("host");
        let got = details(&pb::request_preview::Preview::CrossMachineCopy(copy()), &names);
        let from = got.facts.iter().find(|f| f.label == "From").unwrap();
        let to = got.facts.iter().find(|f| f.label == "To").unwrap();
        assert_eq!(from.lead, "devbox-1");
        assert_eq!(to.lead, "This Mac");
        assert!(from.rest.contains("backoff.rs"));
        assert!(!got.undo_text.is_empty());
        assert!(!got.affects.is_empty());
    }

    #[test]
    fn a_session_move_names_machines_and_what_it_affects() {
        let names = Names::of_host("host").naming("devbox-1", "devbox-1");
        let preview = pb::MovePreview {
            from: Some(pb::MachineId { value: "host".to_owned() }),
            to: Some(pb::MachineId { value: "devbox-1".to_owned() }),
            session_title: "Retry backoff".to_owned(),
            channel_name: "payments".to_owned(),
            repos: vec!["infra".to_owned(), "ledger".to_owned()],
            uncommitted_files: 3,
            terminals: vec!["codex".to_owned(), "zsh running pgbench".to_owned()],
            forks_staying: vec!["Try a queue".to_owned()],
            clones_needed: vec!["ledger".to_owned()],
            ..Default::default()
        };
        let got = details(&pb::request_preview::Preview::Move(preview), &names);
        let from = got.facts.iter().find(|f| f.label == "From").unwrap();
        let to = got.facts.iter().find(|f| f.label == "To").unwrap();
        let carries = got.facts.iter().find(|f| f.label == "Carries").unwrap();
        assert_eq!(from.lead, "This Mac");
        assert_eq!(to.lead, "devbox-1");
        assert!(from.rest.contains("payments") && from.rest.contains("Retry backoff"));
        assert!(carries.lead.contains("ledger") && carries.lead.contains('3'));
        // Terminals are the one effect that is no warning; the clone and the fork are.
        assert_eq!(got.affects.iter().filter(|e| !e.warn).count(), 1);
        assert_eq!(got.affects.iter().filter(|e| e.warn).count(), 2);
        assert!(got.affects.iter().any(|e| e.rest.contains("pgbench") || e.lead.contains("pgbench")));
        assert!(!got.undo_text.is_empty());
    }

    #[test]
    fn a_session_move_with_nothing_known_omits_the_carries_row() {
        let got = details(
            &pb::request_preview::Preview::Move(pb::MovePreview::default()),
            &Names::default(),
        );
        assert!(got.facts.iter().all(|f| f.label != "Carries"));
        assert!(got.affects.is_empty());
    }

    #[test]
    fn a_move_of_files_warns_about_what_it_replaces_and_where_the_folder_goes() {
        let mut preview = copy();
        preview.moves = true;
        preview.files = 3;
        preview.replaced = vec![
            pb::ReplacedFile { path: "a.rs".to_owned(), note: "same content".to_owned() },
            pb::ReplacedFile { path: "b.rs".to_owned(), note: String::new() },
        ];
        let got = details(&pb::request_preview::Preview::CrossMachineCopy(preview), &Names::of_host("host"));
        assert_eq!(got.affects.iter().filter(|e| e.warn).count(), 2);
        let replaced = got.affects.iter().find(|e| e.rest.contains("a.rs")).unwrap();
        assert!(replaced.rest.contains("b.rs"));
        // One file is new.
        assert_eq!(got.affects.iter().filter(|e| !e.warn).count(), 1);
    }

    #[test]
    fn a_plain_copy_replaces_nothing_and_stays_a_single_calm_row() {
        let got = details(&pb::request_preview::Preview::CrossMachineCopy(copy()), &Names::of_host("host"));
        assert!(got.affects.iter().all(|e| !e.warn));
    }

    #[test]
    fn a_preview_with_nothing_to_add_yields_no_rows() {
        let got = details(
            &pb::request_preview::Preview::Question(pb::QuestionPreview::default()),
            &Names::default(),
        );
        assert!(got.facts.is_empty() && got.affects.is_empty() && got.undo_text.is_empty());
    }

    #[test]
    fn the_rule_label_reads_as_the_settings_row() {
        assert_eq!(
            rule_label(Capability::SessionMove),
            "Move a session to another machine"
        );
    }

    #[test]
    fn a_session_on_a_board_item_says_what_it_works_on() {
        let items = [item(&["s1"], true), item(&["s2"], false)];
        assert_eq!(working_on(&items, "s2"), "Working on PAY-3 Ledger schema v2");
        assert_eq!(working_on(&items, "s1"), "");
        assert_eq!(working_on(&items, "s9"), "");
    }

    #[test]
    fn a_file_read_preview_names_source_paths_access_mode_and_calm_effects() {
        let names = Names::of_host("host");
        let preview = pb::FileAccessPreview {
            source_session: Some(pb::SessionId {
                value: "se_other".to_owned(),
            }),
            source_machine: Some(pb::MachineId {
                value: "host".to_owned(),
            }),
            mode: pb::file_access_preview::Mode::ReadThisCall as i32,
            paths: vec![pb::RepoPath {
                repo: Some(pb::RepoRef {
                    owner: "acme".to_owned(),
                    name: "api".to_owned(),
                }),
                path: "src/retry/backoff.rs".to_owned(),
            }],
        };
        let got = details(&pb::request_preview::Preview::FileAccess(preview), &names);
        let from = got.facts.iter().find(|f| f.label == "From").unwrap();
        let files = got.facts.iter().find(|f| f.label == "Files").unwrap();
        let access = got.facts.iter().find(|f| f.label == "Access").unwrap();
        assert_eq!(from.lead, "se_other");
        assert!(from.rest.contains("This Mac"));
        assert_eq!(files.lead, "acme/api/src/retry/backoff.rs");
        assert_eq!(access.lead, "Read only");
        assert!(!got.affects.is_empty());
        assert!(got.affects.iter().all(|effect| !effect.warn));
        assert!(!got.undo_text.is_empty());
    }

    #[test]
    fn file_read_details_resolves_session_title_and_agent_from_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        store
            .put_session(&humata2_store::SessionRecord {
                id: "se_other".to_owned(),
                channel: "ch_pay".to_owned(),
                machine: "devbox-1".to_owned(),
                title: "Refund retries".to_owned(),
                agent: "copilot".to_owned(),
                folder: std::path::PathBuf::from("/work/refund"),
                created_unix_ms: 0,
            })
            .unwrap();
        let place = Place {
            machine: humata2_access::MachineId::new("devbox-1"),
            root: Root::Clone(humata2_access::SessionRef {
                id: humata2_access::SessionId::new("se_other"),
                channel: humata2_access::ChannelId::new("ch_pay"),
                machine: humata2_access::MachineId::new("devbox-1"),
            }),
            path: humata2_access::RepoPath::new("acme/api/src/backoff.rs").unwrap(),
            entry: Entry::File,
        };
        let access = pb::FileAccessPreview {
            source_session: Some(pb::SessionId {
                value: "se_other".to_owned(),
            }),
            source_machine: Some(pb::MachineId {
                value: "devbox-1".to_owned(),
            }),
            mode: pb::file_access_preview::Mode::ReadThisCall as i32,
            paths: vec![pb::RepoPath {
                repo: Some(pb::RepoRef {
                    owner: "acme".to_owned(),
                    name: "api".to_owned(),
                }),
                path: "src/backoff.rs".to_owned(),
            }],
        };
        let names = Names::of_host("host").naming("devbox-1", "devbox-1");
        let got = file_read_details(&access, &place, &store, &names);
        let from = got.facts.iter().find(|f| f.label == "From").unwrap();
        assert_eq!(from.lead, "Refund retries");
        assert!(from.rest.contains("copilot") && from.rest.contains("devbox-1"));
        assert!(got.affects.iter().any(|e| e.lead.contains("Refund retries") && e.rest.contains("copilot")));
        assert!(got.affects.iter().all(|e| !e.warn));
    }
}
