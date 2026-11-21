//! A channel's own Desk sessions (CD-1, CD-3, D306): short-lived managerial
//! sessions of one channel, whose agent uses that channel's rules with the
//! channel as its own. On top of the rules, the user gives each one
//! management switches. A switch is a grant applied after Never, as D182
//! does for the Desk: a rule set to Never still refuses, and a switch that
//! is off adds nothing, so the channel's rule decides.
//!
//! With a session as the target, a switch covers that session and its
//! forks, later ones included, and "Start and fork sessions" covers only
//! forking them. The caller works out that set (`covers`) on every call, so
//! a fork made after the Desk session started is covered from its first
//! call. No switch answers Inbox requests (IN-23): that row is user-only
//! and never reaches a switch.

use std::collections::BTreeSet;

use crate::call::{BoardRef, Call, Root, Target};
use crate::capability::{Capability, ChannelRules, Decision};
use crate::ids::SessionId;

/// What a channel Desk session's agent may do on top of its channel's
/// rules (CD-3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelDeskAccess {
    /// The session it looks after; `None` when it looks after the whole
    /// channel.
    pub target: Option<SessionId>,
    /// The sessions its switches cover: the target and every fork of it,
    /// transitively. `None` covers every session of the channel. Never
    /// holds a channel Desk session: those are filed under no channel.
    pub covers: Option<BTreeSet<SessionId>>,
    /// The switches the user set (IN-23: only the user changes them).
    pub switches: ChannelDeskSwitches,
}

/// The six management switches (CD-3). [`Default`] is what a New desk
/// session dialog starts with: `read_files` and `start_and_fork` on, the rest
/// off (DESKSPAWN).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelDeskSwitches {
    /// "Read its files": the covered sessions' clones, bridged reads
    /// included.
    pub read_files: bool,
    /// "Send notes and questions" to the covered sessions.
    pub send_notes: bool,
    /// "Type into its terminal": typing can answer the target CLI's own
    /// permission prompts.
    pub type_into_terminal: bool,
    /// "Start and fork sessions"; with a session target, only forking it
    /// and its forks.
    pub start_and_fork: bool,
    /// "Stop and archive sessions"; never delete.
    pub stop_and_archive: bool,
    /// "Change anyone's items on the channel board".
    pub change_board_items: bool,
}

impl Default for ChannelDeskSwitches {
    fn default() -> Self {
        Self {
            read_files: true,
            send_notes: false,
            type_into_terminal: false,
            start_and_fork: true,
            stop_and_archive: false,
            change_board_items: false,
        }
    }
}

/// One management switch, by name (CD-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChannelDeskSwitch {
    ReadFiles,
    SendNotes,
    TypeIntoTerminal,
    StartAndFork,
    StopAndArchive,
    ChangeBoardItems,
}

impl ChannelDeskSwitch {
    /// Every switch, in the order the Info panel lists them.
    pub const ALL: [ChannelDeskSwitch; 6] = [
        Self::ReadFiles,
        Self::SendNotes,
        Self::TypeIntoTerminal,
        Self::StartAndFork,
        Self::StopAndArchive,
        Self::ChangeBoardItems,
    ];

    /// The IN-17 rows this switch can allow on top of the channel's rule:
    /// - ReadFiles: FilesReadOtherSession, FilesReadOtherMachine;
    /// - SendNotes: SessionNote;
    /// - TypeIntoTerminal: SessionType;
    /// - StartAndFork: SessionStartSameMachine (DESKSPAWN: same machine only;
    ///   other machine follows the channel rule);
    /// - StopAndArchive: SessionStopOther (archiving another session goes
    ///   through it, D21); deleting another session is never a switch;
    /// - ChangeBoardItems: ChannelBoardEditOthers.
    pub fn capabilities(self) -> &'static [Capability] {
        match self {
            Self::ReadFiles => &[
                Capability::FilesReadOtherSession,
                Capability::FilesReadOtherMachine,
            ],
            Self::SendNotes => &[Capability::SessionNote],
            Self::TypeIntoTerminal => &[Capability::SessionType],
            Self::StartAndFork => &[Capability::SessionStartSameMachine],
            Self::StopAndArchive => &[Capability::SessionStopOther],
            Self::ChangeBoardItems => &[Capability::ChannelBoardEditOthers],
        }
    }

    /// The switches a channel's rules block: those with a row set to Never
    /// among their [`capabilities`](Self::capabilities), whether the switch
    /// is on or off. The Info panel shows "Blocked by the channel rule:
    /// Never" beside each, with a link to that rule (CD-3).
    pub fn blocked_by(rules: &ChannelRules) -> Vec<ChannelDeskSwitch> {
        Self::ALL
            .into_iter()
            .filter(|switch| {
                switch
                    .capabilities()
                    .iter()
                    .any(|capability| rules.decision(*capability) == Some(Decision::Never))
            })
            .collect()
    }
}

/// True when a switch that is on allows `capability` for `call`. The policy
/// asks this only after the Never check, so Never still wins.
///
/// What each covers, with `covered(id)` true when `access.covers` is `None`
/// or holds `id`:
/// - FilesReadOtherSession, FilesReadOtherMachine on `Call::ReadFiles`:
///   `read_files`, and a `Root::Clone` of a covered session; any other root
///   (the channel's `shared/`, main clones or folder on another machine)
///   only with `covers` `None`;
/// - SessionNote on `Target::Session` of a covered session: `send_notes`;
/// - SessionType on `Target::Session` of a covered session:
///   `type_into_terminal`;
/// - SessionStopOther on `Target::Session` of a covered session:
///   `stop_and_archive`;
/// - SessionStartSameMachine, SessionStartOtherMachine on `Target::Start`:
///   `start_and_fork`; with `covers` `None` any start or fork, otherwise
///   only a fork (`fork_of`) of a covered session;
/// - ChannelBoardEditOthers on `Call::Board`: `change_board_items`; the
///   channel's board only with `covers` `None`, a session's board when that
///   session is covered, never the Global board;
/// - anything else: false.
pub(crate) fn switch_allows(access: &ChannelDeskAccess, capability: Capability, call: &Call) -> bool {
    let switches = &access.switches;
    let covered = |session: &SessionId| {
        access
            .covers
            .as_ref()
            .is_none_or(|covers| covers.contains(session))
    };
    match (capability, call) {
        (
            Capability::FilesReadOtherSession | Capability::FilesReadOtherMachine,
            Call::ReadFiles(place),
        ) => {
            switches.read_files
                && match &place.root {
                    Root::Clone(owner) => covered(&owner.id),
                    Root::Shared(_)
                    | Root::ChannelFolder(_)
                    | Root::MainClone { .. } => access.covers.is_none(),
                    Root::Home | Root::Pastes(_) | Root::AgentState(_) | Root::Received | Root::Outside => false,
                }
        }
        (Capability::SessionNote, Call::Use { target: Target::Session(session), .. }) => {
            switches.send_notes && covered(&session.id)
        }
        (Capability::SessionType, Call::Use { target: Target::Session(session), .. }) => {
            switches.type_into_terminal && covered(&session.id)
        }
        (Capability::SessionStopOther, Call::Use { target: Target::Session(session), .. }) => {
            switches.stop_and_archive && covered(&session.id)
        }
        (
            Capability::SessionStartSameMachine,
            Call::Use {
                target: Target::Start { fork_of, .. },
                ..
            },
        ) => {
            switches.start_and_fork
                && match (&access.covers, fork_of) {
                    (None, _) => true,
                    (Some(covers), Some(parent)) => covers.contains(parent),
                    (Some(_), None) => false,
                }
        }
        (Capability::ChannelBoardEditOthers, Call::Board { board, .. }) => {
            switches.change_board_items
                && match board {
                    BoardRef::Channel(_) => access.covers.is_none(),
                    BoardRef::Session(session) => covered(&session.id),
                    BoardRef::Global => false,
                }
        }
        _ => false,
    }
}

/// Machine-readable refusal code when a Desk agent attempts to type into a
/// terminal that is waiting on a permission prompt.
pub const PERMISSION_PROMPT_PROTECTED_CODE: &str = "permission_prompt_protected";

/// Exit code returned when terminal typing is refused because the target
/// session is waiting on a permission prompt.
pub const PERMISSION_PROMPT_EXIT_CODE: i32 = 4;

/// Hard safety refusal for typing into another session's terminal.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TerminalTypeRefusal {
    #[error(
        "Refused (exit 4). {target_title} is waiting on a permission prompt — only the owner may answer it."
    )]
    PermissionPromptProtected { target_title: String },
}

/// Hard safety check before typing into a target session's terminal: if the
/// target is waiting on a permission prompt, refuse with exit 4.
pub fn check_terminal_type_safety(
    target_awaiting_permission_prompt: bool,
    target_title: &str,
) -> Result<(), TerminalTypeRefusal> {
    if target_awaiting_permission_prompt {
        return Err(TerminalTypeRefusal::PermissionPromptProtected {
            target_title: target_title.to_owned(),
        });
    }
    Ok(())
}

/// True when `capability` requires explicit user consent because the
/// corresponding channel Desk switch is off. Never allows `InboxAnswer`.
pub fn requires_explicit_consent(access: &ChannelDeskAccess, capability: Capability) -> bool {
    match capability {
        Capability::SessionNote => !access.switches.send_notes,
        Capability::SessionType => !access.switches.type_into_terminal,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{AgentId, ChannelId, MachineId, SessionRef};

    fn whole_channel_access(switches: ChannelDeskSwitches) -> ChannelDeskAccess {
        ChannelDeskAccess {
            target: None,
            covers: None,
            switches,
        }
    }

    fn start_call(fork_of: Option<SessionId>) -> Call {
        Call::Use {
            capability: Capability::SessionStartSameMachine,
            target: Target::Start {
                channel: ChannelId::new("ch_1"),
                machine: MachineId::new("m_1"),
                agent: AgentId::new("claude"),
                fork_of,
            },
        }
    }

    fn session_call(capability: Capability) -> Call {
        Call::Use {
            capability,
            target: Target::Session(SessionRef {
                id: SessionId::new("se_1"),
                channel: ChannelId::new("ch_1"),
                machine: MachineId::new("m_1"),
            }),
        }
    }

    #[test]
    fn default_switches_match_deskspawn() {
        let switches = ChannelDeskSwitches::default();
        assert!(switches.read_files);
        assert!(!switches.send_notes);
        assert!(!switches.type_into_terminal);
        assert!(switches.start_and_fork);
        assert!(!switches.stop_and_archive);
        assert!(!switches.change_board_items);
    }

    #[test]
    fn start_and_fork_allows_same_machine_not_other_machine() {
        let access = whole_channel_access(ChannelDeskSwitches::default());
        assert!(switch_allows(
            &access,
            Capability::SessionStartSameMachine,
            &start_call(None),
        ));
        assert!(!switch_allows(
            &access,
            Capability::SessionStartOtherMachine,
            &start_call(None),
        ));
    }

    #[test]
    fn note_and_type_require_explicit_consent_by_default() {
        let access = whole_channel_access(ChannelDeskSwitches::default());
        assert!(requires_explicit_consent(&access, Capability::SessionNote));
        assert!(requires_explicit_consent(&access, Capability::SessionType));
        assert!(!switch_allows(
            &access,
            Capability::SessionNote,
            &session_call(Capability::SessionNote),
        ));
        assert!(!switch_allows(
            &access,
            Capability::SessionType,
            &session_call(Capability::SessionType),
        ));
        assert!(!requires_explicit_consent(&access, Capability::InboxAnswer));
        assert!(!switch_allows(
            &access,
            Capability::InboxAnswer,
            &session_call(Capability::InboxAnswer),
        ));
    }

    #[test]
    fn terminal_type_safety_hard_refuses_permission_prompt() {
        assert_eq!(PERMISSION_PROMPT_PROTECTED_CODE, "permission_prompt_protected");
        assert_eq!(PERMISSION_PROMPT_EXIT_CODE, 4);
        let err = check_terminal_type_safety(true, "Restart doctor").unwrap_err();
        assert_eq!(
            err,
            TerminalTypeRefusal::PermissionPromptProtected {
                target_title: "Restart doctor".to_owned(),
            }
        );
        assert!(err.to_string().contains("exit 4"));
        assert!(err.to_string().contains("Restart doctor"));
        assert_eq!(check_terminal_type_safety(false, "Restart doctor"), Ok(()));
    }

    #[test]
    fn never_rule_blocks_start_and_fork_switch() {
        let mut rules = ChannelRules::defaults();
        rules.set_decision(Capability::SessionStartSameMachine, Decision::Never);
        let blocked = ChannelDeskSwitch::blocked_by(&rules);
        assert!(blocked.contains(&ChannelDeskSwitch::StartAndFork));
    }
}
