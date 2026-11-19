//! PER-28: the two box power cuts during Start that left something behind
//! in the full suite (power_cut_machine), one fast case each. A fresh host
//! and box, the box's daemon cut at the point while it starts a session,
//! both started again, then the suite's checks: no host start journal left
//! in `carry/remote_archive/start`, and no half-made `.partial` clone or
//! folder.

use humatad_test_common as common;
use humatad_test_common::remote_rig;
use humatad_test_common::power_cut;

use power_cut::{Cut, Operation, Side};

async fn cut_at(point: &str) {
    let problems = power_cut::run_case(Cut::Machine, Operation::Start, Side::Box, point).await;
    assert!(problems.is_empty(), "{} problem(s):\n{}", problems.len(), problems.join("\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_cut_on_the_box_while_it_writes_a_file_leaves_no_host_start_journal() {
    cut_at("store.file.temp_written").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_cut_on_the_box_while_it_keeps_the_session_detail_leaves_no_partial_clone() {
    cut_at("store.session_detail.put.committing").await;
}
