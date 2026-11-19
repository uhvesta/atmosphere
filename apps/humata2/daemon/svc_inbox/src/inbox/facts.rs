//! What an agent's Inbox card tells the owner besides the action itself
//! (INBOXHOOKS): the From/To/Carries rows, what else the action affects,
//! whether it can be undone, which board item the agent is on, and the rule
//! that asked. All of it is worked out here from what the card already
//! holds, in names rather than ids.

use humata2_access::Capability;
use humata2_daemon_ports::Names;
use humata2_grpc::humata::v1 as pb;
use humata2_store::BoardItemRecord;

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
        _ => Details::default(),
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
    Details {
        facts: vec![
            fact("From", source.clone(), format!("· {}", copy.source_path.replace('\n', ", "))),
            fact("To", destination, format!("· {landing}")),
            fact("Carries", size, String::new()),
        ],
        affects: vec![pb::CardEffect {
            warn: false,
            lead: format!("Nothing changes on {source}"),
            rest: "; the files are copied, not moved".to_owned(),
        }],
        undo_text: "By hand: delete the copied files. The originals are untouched."
            .to_owned(),
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
}
