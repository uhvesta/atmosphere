//! PER-28, D243: the power is cut at each fault point an operation reaches on either side, and on the other side at once after, with the holders of both. After both start again and the box links, nothing PER-25 lists is lost: each item is as it was or finished, or a restart report names it. An operation is one test, its points cut a few at a time (see tests/common/power_cut.rs), and Bazel shards the tests.

use humatad_test_common as common;
use humatad_test_common::remote_rig;
use humatad_test_common::power_cut;

use power_cut::{Cut, Operation};

/// One test for each operation.
macro_rules! cut_tests {
    ($($name:ident: $operation:expr;)*) => {
        $(
            #[tokio::test(flavor = "multi_thread")]
            async fn $name() {
                power_cut::run_cases(Cut::Both, $operation).await;
            }
        )*
    };
}

cut_tests! {
    a_start_with_both_machines_power_cut_loses_nothing: Operation::Start;
    a_local_fork_with_both_machines_power_cut_loses_nothing: Operation::LocalFork;
    a_fork_onto_the_box_with_both_machines_power_cut_loses_nothing: Operation::RemoteFork;
    a_move_with_both_machines_power_cut_loses_nothing: Operation::Move;
    an_archive_on_the_box_with_both_machines_power_cut_loses_nothing: Operation::RemoteArchive;
    a_restore_on_the_host_with_both_machines_power_cut_loses_nothing: Operation::RemoteRestore;
    a_comment_send_with_both_machines_power_cut_loses_nothing: Operation::CommentSend;
    a_transfer_with_both_machines_power_cut_loses_nothing: Operation::Transfer;
    a_host_variable_on_a_box_with_both_machines_power_cut_loses_nothing: Operation::RemoteHostVariable;
}
