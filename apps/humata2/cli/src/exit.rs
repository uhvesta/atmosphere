// VERIFY dummy comment: closed without merging.
//! Exit codes (CLI.md §7, D26). Scripts branch on these or on
//! `error.kind`, never on the text.

use humata2_proto::humata::v1 as pb;
use pb::error::Kind;
use tonic::Code;

pub const OK: i32 = 0;
pub const USAGE: i32 = 2;
pub const NOT_FOUND: i32 = 3;
pub const DENIED: i32 = 4;
pub const NEEDS_USER: i32 = 5;
pub const CONFLICT: i32 = 6;
pub const PRECONDITION: i32 = 7;
pub const UNREACHABLE: i32 = 8;
pub const TOOL_FAILED: i32 = 9;
pub const RESOURCES: i32 = 10;
pub const DATA: i32 = 11;
pub const PENDING: i32 = 12;
pub const JOURNALED_STRICT: i32 = 20;
pub const INTERNAL: i32 = 70;
pub const INTERRUPTED: i32 = 130;

/// The kind's name, as `--json` prints it, and its exit code. Every kind
/// is listed, so a new one can't compile without a code.
pub fn of_kind(kind: &Kind) -> (&'static str, i32) {
    match kind {
        Kind::NotFound(_) => ("not_found", NOT_FOUND),
        Kind::MachineOffline(_) => ("machine_offline", UNREACHABLE),
        Kind::HostUnreachable(_) => ("host_unreachable", UNREACHABLE),
        Kind::Denied(_) => ("denied", DENIED),
        Kind::UserOnly(_) => ("user_only", NEEDS_USER),
        Kind::ApprovalPending(_) => ("pending", PENDING),
        Kind::PresenceRequired(_) => ("presence_required", NEEDS_USER),
        Kind::IdentityMismatch(_) => ("identity_mismatch", NEEDS_USER),
        Kind::PeerUnproven(_) => ("peer_unproven", DENIED),
        Kind::RevisionConflict(_) => ("revision_conflict", CONFLICT),
        Kind::PlanStale(_) => ("plan_stale", CONFLICT),
        Kind::NameTaken(_) => ("name_taken", CONFLICT),
        Kind::InvalidField(_) => ("invalid_field", USAGE),
        Kind::UnpushedWork(_) => ("unpushed_work", PRECONDITION),
        Kind::SessionBusy(_) => ("session_busy", PRECONDITION),
        Kind::ParentReadOnly(_) => ("parent_read_only", DENIED),
        Kind::MainCloneReadOnly(_) => ("main_clone_read_only", DENIED),
        Kind::OutsideHumataFolders(_) => ("outside_humata_folders", USAGE),
        Kind::DiskFull(_) => ("disk_full", RESOURCES),
        Kind::RateLimited(_) => ("rate_limited", RESOURCES),
        Kind::GithubCli(_) => ("github_cli", TOOL_FAILED),
        Kind::GitFailed(_) => ("git_failed", TOOL_FAILED),
        Kind::PushRefused(_) => ("push_refused", DENIED),
        Kind::SshFailed(_) => ("ssh_failed", UNREACHABLE),
        Kind::HostKeyChanged(_) => ("host_key_changed", PRECONDITION),
        Kind::ChecksumMismatch(_) => ("checksum_mismatch", DATA),
        Kind::ResumeFailed(_) => ("resume_failed", PRECONDITION),
        Kind::AgentLacksFeature(_) => ("agent_lacks_feature", PRECONDITION),
        Kind::ProviderStepFailed(_) => ("provider_step_failed", TOOL_FAILED),
        Kind::ProviderIdentityInvalid(_) => ("provider_identity_invalid", PRECONDITION),
        Kind::LoginNeedsUser(_) => ("login_needs_user", PRECONDITION),
        Kind::PeerVersionUnsupported(_) => ("peer_version_unsupported", PRECONDITION),
        Kind::OffsetTrimmed(_) => ("offset_trimmed", DATA),
        Kind::NotCancellable(_) => ("not_cancellable", PRECONDITION),
        Kind::AlreadyAnswered(_) => ("already_answered", CONFLICT),
        Kind::RequestWithdrawn(_) => ("request_withdrawn", PRECONDITION),
        Kind::UserAway(_) => ("user_away", PRECONDITION),
        Kind::NotInChannel(_) => ("not_in_channel", DENIED),
        Kind::PrClosed(_) => ("pr_closed", PRECONDITION),
        Kind::ReviewPartlyPosted(_) => ("review_partly_posted", TOOL_FAILED),
        Kind::SkillNameClash(_) => ("skill_name_clash", PRECONDITION),
        Kind::LingerUnavailable(_) => ("linger_unavailable", PRECONDITION),
        Kind::DaemonAlreadyRunning(_) => ("daemon_already_running", PRECONDITION),
        Kind::DaemonNotRunning(_) => ("daemon_not_running", PRECONDITION),
        Kind::JournalEntryRejected(_) => ("journal_entry_rejected", PRECONDITION),
        Kind::RepoNotVisible(_) => ("repo_not_visible", PRECONDITION),
        Kind::StepFailed(_) => ("step_failed", TOOL_FAILED),
        Kind::PassphraseLocked(_) => ("passphrase_locked", RESOURCES),
        Kind::DependencyLoop(_) => ("dependency_loop", PRECONDITION),
        Kind::Internal(_) => ("internal", INTERNAL),
    }
}

/// A status with no Humata error in it, such as UNIMPLEMENTED from a
/// service humatad2 hasn't built yet: named after its gRPC code.
pub fn of_status(code: i32) -> (&'static str, i32) {
    match Code::from_i32(code) {
        Code::Ok => ("ok", OK),
        Code::Cancelled => ("cancelled", INTERRUPTED),
        Code::Unknown => ("unknown", INTERNAL),
        Code::InvalidArgument => ("invalid_argument", USAGE),
        Code::DeadlineExceeded => ("deadline_exceeded", UNREACHABLE),
        Code::NotFound => ("not_found", NOT_FOUND),
        Code::AlreadyExists => ("already_exists", CONFLICT),
        Code::PermissionDenied => ("permission_denied", DENIED),
        Code::ResourceExhausted => ("resource_exhausted", RESOURCES),
        Code::FailedPrecondition => ("failed_precondition", PRECONDITION),
        Code::Aborted => ("aborted", CONFLICT),
        Code::OutOfRange => ("out_of_range", USAGE),
        Code::Unimplemented => ("unimplemented", PRECONDITION),
        Code::Internal => ("internal", INTERNAL),
        Code::Unavailable => ("unavailable", UNREACHABLE),
        Code::DataLoss => ("data_loss", DATA),
        Code::Unauthenticated => ("unauthenticated", NEEDS_USER),
    }
}
