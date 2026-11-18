//! PER-28, D243: the box's power is cut at each fault point an operation reaches on the box, with its holders; the host keeps running. After the box starts again and links, nothing PER-25 lists is lost: each item is as it was or finished, or a restart report names it. An operation is one test, its points cut a few at a time (see tests/common/power_cut.rs), and Bazel shards the tests.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;
#[path = "common/power_cut.rs"]
mod power_cut;

use power_cut::{Cut, Operation};

/// One test for each operation.
macro_rules! cut_tests {
    ($($name:ident: $operation:expr;)*) => {
        $(
            #[tokio::test(flavor = "multi_thread")]
            async fn $name() {
                power_cut::run_cases(Cut::Machine, $operation).await;
            }
        )*
    };
}

cut_tests! {
    a_start_with_a_boxs_power_cut_loses_nothing: Operation::Start;
    a_local_fork_with_a_boxs_power_cut_loses_nothing: Operation::LocalFork;
    a_fork_onto_the_box_with_a_boxs_power_cut_loses_nothing: Operation::RemoteFork;
    a_move_with_a_boxs_power_cut_loses_nothing: Operation::Move;
    an_archive_on_the_box_with_a_boxs_power_cut_loses_nothing: Operation::RemoteArchive;
    a_restore_on_the_host_with_a_boxs_power_cut_loses_nothing: Operation::RemoteRestore;
    a_comment_send_with_a_boxs_power_cut_loses_nothing: Operation::CommentSend;
    a_transfer_with_a_boxs_power_cut_loses_nothing: Operation::Transfer;
    a_host_variable_on_a_box_with_a_boxs_power_cut_loses_nothing: Operation::RemoteHostVariable;
}
