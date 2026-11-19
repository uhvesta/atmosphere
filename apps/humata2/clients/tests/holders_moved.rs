//! The "3 of 7 terminals moved" line a Holders Migrate shows while it runs.

use humata2_clients::model::HoldersMoved;

fn moved(done: u32, total: u32) -> HoldersMoved {
    HoldersMoved { done, total, started_ms: 10 }
}

#[test]
fn a_round_with_nothing_moved_reads_empty_and_measured() {
    let progress = moved(0, 7).progress();
    assert_eq!(progress.fraction, Some(0.0));
    assert_eq!(progress.label, "0 of 7 terminals moved");
    assert_eq!(progress.started_ms, 10);
}

#[test]
fn part_way_reads_the_share_moved() {
    let progress = moved(3, 7).progress();
    assert_eq!(progress.fraction, Some(3.0 / 7.0));
    assert_eq!(progress.label, "3 of 7 terminals moved");
}

#[test]
fn all_moved_is_full_and_one_terminal_is_singular() {
    assert_eq!(moved(7, 7).progress().fraction, Some(1.0));
    assert_eq!(moved(1, 1).progress().label, "1 of 1 terminal moved");
}

#[test]
fn an_unknown_total_is_unmeasured() {
    assert_eq!(moved(0, 0).progress().fraction, None);
}

#[test]
fn more_done_than_total_is_capped_at_full() {
    let progress = moved(9, 7).progress();
    assert_eq!(progress.fraction, Some(1.0));
    assert_eq!(progress.label, "7 of 7 terminals moved");
}
