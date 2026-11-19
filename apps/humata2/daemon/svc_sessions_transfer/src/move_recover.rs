//! After a restart: the moves it cut short (MV-14, flow 26.6). Each journal
//! `<state>/carry/<operation>.json` names a session; from the moment the host
//! starts, that session shows Moving. Once the start's own recovery is done
//! and the links of the move's two machines are up, the move is undone (the
//! agent still runs on the old machine) or finished (it runs on the new
//! one), and the session shows what it is again.

use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tonic::Status;

use crate::SessionTransferSvc;
use crate::ports::{LinkState, chain};

/// A move journal persisted under `<state>/carry/<operation>.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveJournal {
    pub operation: String,
    pub session: String,
    pub from: String,
    pub to: String,
    pub phase: String,
    #[serde(default)]
    pub agent_ran: bool,
    #[serde(default)]
    pub frozen: bool,
}

/// The remote machines of a move and the generation of each one's link
/// when a try began (none when it had dropped again by then).
pub type Seen = Vec<(String, Option<u64>)>;

/// Outcome of a single move-recovery attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryOutcome {
    Finished,
    Undone { reason: String },
    Stuck { reason: String },
}

/// Classifies the outcome of a move-recovery attempt given the journal phase.
pub fn classify_recovery_outcome(
    phase: &str,
    from: &str,
    result: Result<(), &Status>,
) -> RecoveryOutcome {
    match result {
        // The agent runs on the new machine from `resumed` on.
        Ok(()) if matches!(phase, "resumed" | "switched") => RecoveryOutcome::Finished,
        Ok(()) => RecoveryOutcome::Undone {
            reason: format!("the agent still ran on {from}"),
        },
        Err(error) => RecoveryOutcome::Stuck {
            reason: chain(error),
        },
    }
}

/// Reads all valid [`MoveJournal`] files in `<state_dir>/carry/`.
pub fn read_journals(state_dir: &Path) -> Vec<MoveJournal> {
    let dir = state_dir.join("carry");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut journals = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if let Ok(journal) = serde_json::from_slice::<MoveJournal>(&bytes) {
            journals.push(journal);
        }
    }
    journals
}

/// Waits on `rx` until `machine`'s link is reported `Up`, returning its generation.
pub async fn wait_link_up(
    rx: &mut broadcast::Receiver<(String, LinkState)>,
    machine: &str,
) -> Result<u64, ()> {
    loop {
        match rx.recv().await {
            Ok((m, LinkState::Up { generation })) if m == machine => return Ok(generation),
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return Err(()),
        }
    }
}

/// Waits on `rx` until `machine`'s link is reported `Up` with a generation
/// different from `seen`.
pub async fn wait_link_came_back(
    rx: &mut broadcast::Receiver<(String, LinkState)>,
    machine: &str,
    seen: Option<u64>,
) -> Result<u64, ()> {
    loop {
        match rx.recv().await {
            Ok((m, LinkState::Up { generation }))
                if m == machine && Some(generation) != seen =>
            {
                return Ok(generation);
            }
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return Err(()),
        }
    }
}

impl SessionTransferSvc {
    /// Reads move journals left by the previous run and marks non-aborted
    /// sessions as moving via `TransferEnv`.
    pub fn mark_pending_move_journals(&self) -> Vec<MoveJournal> {
        let journals = read_journals(&self.env.state_dir());
        for journal in &journals {
            if journal.phase != "aborted" {
                self.env.set_moving(&journal.session, true);
            }
        }
        journals
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_journal_roundtrips_json() {
        let journal = MoveJournal {
            operation: "op_1".to_owned(),
            session: "se_1".to_owned(),
            from: "m1".to_owned(),
            to: "m2".to_owned(),
            phase: "unpacked".to_owned(),
            agent_ran: true,
            frozen: true,
        };
        let dir = tempfile::tempdir().unwrap();
        let carry = dir.path().join("carry");
        std::fs::create_dir_all(&carry).unwrap();
        std::fs::write(
            carry.join("op_1.json"),
            serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();
        let loaded = read_journals(dir.path());
        assert_eq!(loaded, vec![journal]);
    }

    #[test]
    fn classify_recovery_outcome_matches_phase() {
        assert_eq!(
            classify_recovery_outcome("resumed", "m1", Ok(())),
            RecoveryOutcome::Finished
        );
        assert_eq!(
            classify_recovery_outcome("switched", "m1", Ok(())),
            RecoveryOutcome::Finished
        );
        assert_eq!(
            classify_recovery_outcome("unpacked", "m1", Ok(())),
            RecoveryOutcome::Undone {
                reason: "the agent still ran on m1".to_owned()
            }
        );
        let err = Status::unavailable("link down");
        assert!(matches!(
            classify_recovery_outcome("copied", "m1", Err(&err)),
            RecoveryOutcome::Stuck { .. }
        ));
    }

    #[tokio::test]
    async fn wait_link_up_and_came_back_observe_generations() {
        let (tx, mut rx) = broadcast::channel(8);
        tx.send(("m1".to_owned(), LinkState::Down)).unwrap();
        tx.send(("m1".to_owned(), LinkState::Up { generation: 1 })).unwrap();
        tx.send(("m1".to_owned(), LinkState::Up { generation: 2 })).unwrap();

        let gen1 = wait_link_up(&mut rx, "m1").await.unwrap();
        assert_eq!(gen1, 1);
        let gen2 = wait_link_came_back(&mut rx, "m1", Some(1)).await.unwrap();
        assert_eq!(gen2, 2);
    }
}
