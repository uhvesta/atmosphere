//! A channel setup (frame 45): the daemon's operation steps as the app's
//! setup steps. Pure: no calls.

use humata2_clients::model::{ChannelSetup, MachineId, SetupId, SetupState, SetupStep};
use humata2_grpc::humata::v1 as pb;

/// The prefix of a step that clones one repository.
const CLONE: &str = "clone_main:";

/// The operation as a setup of channel `name`, whose steps run on `machine`.
pub(crate) fn setup(name: &str, operation: &pb::Operation, machine: &MachineId) -> ChannelSetup {
    ChannelSetup {
        id: SetupId(
            operation
                .id
                .as_ref()
                .map(|id| id.value.clone())
                .unwrap_or_default(),
        ),
        name: name.to_owned(),
        steps: operation.steps.iter().map(|step| step_of(step, machine)).collect(),
        cancellable: operation.cancellable && operation.state == pb::OperationState::Running as i32,
    }
}

/// One daemon step: `record` makes the folders, `clone_main:owner/name` clones.
pub(crate) fn step_of(step: &pb::OperationStep, machine: &MachineId) -> SetupStep {
    SetupStep {
        machine: machine.clone(),
        repo: step.key.strip_prefix(CLONE).map(str::to_owned),
        state: state_of(step),
    }
}

fn state_of(step: &pb::OperationStep) -> SetupState {
    use pb::StepState as S;
    match S::try_from(step.state).unwrap_or(S::Unspecified) {
        S::Running => SetupState::Running {
            percent: percent(step.done_units, step.total_units),
            detail: step.title.clone(),
        },
        S::Done | S::Skipped => SetupState::Done,
        S::Failed => SetupState::Failed {
            reason: step
                .output_tail
                .lines()
                .rev()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| step.title.clone()),
        },
        S::Waiting | S::Undone | S::Unspecified => SetupState::Waiting,
    }
}

fn percent(done: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    let value = (u128::from(done) * 100 / u128::from(total)).min(100);
    u8::try_from(value).unwrap_or(100)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(key: &str, state: pb::StepState, done: u64, total: u64, tail: &str) -> pb::OperationStep {
        pb::OperationStep {
            key: key.to_owned(),
            title: format!("title of {key}"),
            state: state as i32,
            done_units: done,
            total_units: total,
            output_tail: tail.to_owned(),
        }
    }

    #[test]
    fn the_record_step_is_folders_and_a_clone_step_names_its_repo() {
        let machine = MachineId::new("ma_1");
        let folders = step_of(&step("record", pb::StepState::Done, 0, 0, ""), &machine);
        assert_eq!(folders.repo, None);
        assert_eq!(folders.state, SetupState::Done);
        let clone = step_of(&step("clone_main:acme/app", pb::StepState::Waiting, 0, 0, ""), &machine);
        assert_eq!(clone.repo.as_deref(), Some("acme/app"));
        assert_eq!(clone.state, SetupState::Waiting);
        assert_eq!(clone.machine, machine);
    }

    #[test]
    fn a_running_clone_reports_percent_and_what_it_does() {
        let machine = MachineId::new("ma_1");
        let running = step_of(&step("clone_main:a/b", pb::StepState::Running, 3, 4, ""), &machine);
        assert_eq!(
            running.state,
            SetupState::Running {
                percent: 75,
                detail: "title of clone_main:a/b".to_owned()
            }
        );
        let unknown = step_of(&step("clone_main:a/b", pb::StepState::Running, 5, 0, ""), &machine);
        assert!(matches!(unknown.state, SetupState::Running { percent: 0, .. }));
        let over = step_of(&step("clone_main:a/b", pb::StepState::Running, 9, 4, ""), &machine);
        assert!(matches!(over.state, SetupState::Running { percent: 100, .. }));
    }

    #[test]
    fn a_failed_step_gives_the_last_output_line_or_its_title() {
        let machine = MachineId::new("ma_1");
        let with_tail = step_of(
            &step("clone_main:a/b", pb::StepState::Failed, 0, 0, "Cloning...\nfatal: repository not found\n\n"),
            &machine,
        );
        assert_eq!(
            with_tail.state,
            SetupState::Failed {
                reason: "fatal: repository not found".to_owned()
            }
        );
        let bare = step_of(&step("clone_main:a/b", pb::StepState::Failed, 0, 0, ""), &machine);
        assert_eq!(
            bare.state,
            SetupState::Failed {
                reason: "title of clone_main:a/b".to_owned()
            }
        );
    }

    #[test]
    fn cancel_is_offered_only_while_running_and_cancellable() {
        let machine = MachineId::new("ma_1");
        let mut operation = pb::Operation {
            id: Some(pb::OperationId {
                value: "op_1".to_owned(),
            }),
            state: pb::OperationState::Running as i32,
            cancellable: true,
            steps: vec![step("record", pb::StepState::Running, 0, 0, "")],
            ..Default::default()
        };
        let running = setup("web", &operation, &machine);
        assert!(running.cancellable);
        assert_eq!(running.id, SetupId("op_1".to_owned()));
        assert_eq!(running.steps.len(), 1);
        operation.state = pb::OperationState::Failed as i32;
        assert!(!setup("web", &operation, &machine).cancellable);
    }
}
