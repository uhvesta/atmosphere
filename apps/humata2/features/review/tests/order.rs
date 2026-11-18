//! Bottom-up reading order of a review's files (§26.17 step 4).
use humata2_review::order::{OrderFile, bottom_up};

fn file<'a>(path: &'a str, text: &'a str) -> OrderFile<'a> {
    OrderFile { path, text }
}

#[test]
fn a_file_comes_after_the_files_it_uses() {
    let files = [
        file("src/handler.rs", "use crate::refund;\nrefund::issue(cents)"),
        file("src/refund.rs", "let cents = amount.round();"),
    ];
    assert_eq!(bottom_up(&files), vec![1, 0]);
}

#[test]
fn chains_read_from_the_leaf_up() {
    let files = [
        file("src/api.rs", "handler::run()"),
        file("src/handler.rs", "refund::issue()"),
        file("src/refund.rs", "amount.round()"),
    ];
    assert_eq!(bottom_up(&files), vec![2, 1, 0]);
}

#[test]
fn files_that_use_nothing_keep_their_order() {
    let files = [file("a_one.rs", "x"), file("b_two.rs", "y"), file("c_three.rs", "z")];
    assert_eq!(bottom_up(&files), vec![0, 1, 2]);
}

#[test]
fn files_that_use_each_other_keep_their_order_and_none_is_lost() {
    let files = [
        file("src/alpha.rs", "beta::x()"),
        file("src/beta.rs", "alpha::y()"),
        file("src/gamma.rs", "alpha::z()"),
    ];
    let order = bottom_up(&files);
    assert_eq!(order, vec![0, 1, 2]);
}

#[test]
fn a_name_inside_another_word_is_not_a_use() {
    let files = [
        file("src/user.rs", "superuser_refund()"),
        file("src/refund.rs", "x"),
    ];
    assert_eq!(bottom_up(&files), vec![0, 1]);
}

#[test]
fn nothing_gives_nothing() {
    assert!(bottom_up(&[]).is_empty());
}
