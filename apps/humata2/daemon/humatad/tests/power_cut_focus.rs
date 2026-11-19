//! PER-28, D243: single power cuts from the suites in tests/power_cut_*.rs,
//! one test each, for the points whose recovery went wrong once. Each runs
//! one case (no trace, no chunk): a fresh host and box, the operation cut at
//! the point, both started again, and the same checks as the suites. Quick
//! enough to queue for routine checks; the suites stay the full promise.

use humatad_test_common as common;
use humatad_test_common::remote_rig;
use humatad_test_common::power_cut;

use power_cut::{Cut, Operation, Side};

/// One test per (cut, operation, side cut at the point, point).
macro_rules! focus_tests {
    ($($name:ident: $cut:expr, $operation:expr, $origin:expr, $point:literal;)*) => {
        $(
            #[tokio::test(flavor = "multi_thread")]
            async fn $name() {
                let problems = power_cut::run_case($cut, $operation, $origin, $point).await;
                assert!(problems.is_empty(), "{} problem(s):\n{}", problems.len(), problems.join("\n"));
            }
        )*
    };
}

focus_tests! {
    // The host's move journal waited on a resume nothing asked again.
    a_move_cut_on_the_host_after_its_report_of_the_box_finishes:
        Cut::Host, Operation::Move, Side::Host, "store.remote_report.put.committed";
    a_move_cut_on_the_host_while_it_keeps_the_box_report_finishes:
        Cut::Host, Operation::Move, Side::Host, "store.remote_report.put.committing";
    // The host read its view before the box's first report over the new link.
    a_start_on_the_box_cut_on_both_after_the_host_kept_its_report_shows_it_live:
        Cut::Both, Operation::Start, Side::Host, "store.remote_report.put.committed";
    // The host's remote start journal sat in a folder that was never synced.
    a_start_cut_on_both_at_the_box_keeps_the_host_start_journal:
        Cut::Both, Operation::Start, Side::Box, "store.session.put.committing";
    // The host's archive follow-up journal sat in an unsynced folder.
    an_archive_cut_on_both_after_the_box_archived_is_told_to_the_host:
        Cut::Both, Operation::RemoteArchive, Side::Box, "archive.archived";
    an_archive_cut_on_the_host_after_it_asked_the_box_is_followed_up:
        Cut::Host, Operation::RemoteArchive, Side::Host, "archive_remote.asked";
    // The restore journal was lost, or went before the agent's resume.
    a_restore_cut_on_both_while_the_host_keeps_the_session_finishes_or_undoes:
        Cut::Both, Operation::RemoteRestore, Side::Host, "store.session.put.committing";
    // The staged copy of a fork or move, mid transfer.
    a_fork_onto_the_box_cut_on_the_host_while_its_copy_commits_is_whole:
        Cut::Host, Operation::RemoteFork, Side::Host, "transfer.committing";
    a_move_cut_on_the_host_while_its_copy_commits_is_whole:
        Cut::Host, Operation::Move, Side::Host, "transfer.committing";
    // The box cut alone, while it keeps the moved session: the host redials
    // it and the move finishes or is undone (power_cut_machine's path).
    a_move_cut_on_the_box_while_it_keeps_the_session_finishes_or_undoes:
        Cut::Machine, Operation::Move, Side::Box, "store.session.put.committing";
}
