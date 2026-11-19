//! Looks good (D852) kept in the repo, next to the reviewed bookmark.

mod support;

use humata2_review_diff::Look;
use support::{Repo, SESSION};

fn look(path: &str, fingerprint: u64) -> Look {
    Look {
        path: path.to_owned(),
        fingerprint,
    }
}

#[test]
fn saved_looks_read_back_from_a_reopened_store() {
    let repo = Repo::new();
    repo.write("a.txt", "one\n");
    repo.commit_all("base");
    let looks = vec![look("a.txt", u64::MAX), look("dir/with space\tand tab.txt", 7)];
    repo.checkout().save_looks_blocking(SESSION, &looks).expect("save");
    assert_eq!(repo.checkout().looks_blocking(SESSION).expect("read"), looks);
}

#[test]
fn an_unknown_session_reads_no_looks() {
    let repo = Repo::new();
    repo.write("a.txt", "one\n");
    repo.commit_all("base");
    repo.checkout().save_looks_blocking(SESSION, &[look("a.txt", 1)]).expect("save");
    assert!(repo.checkout().looks_blocking("s-other").expect("read").is_empty());
}

#[test]
fn saving_replaces_the_set_and_an_empty_set_forgets_it() {
    let repo = Repo::new();
    repo.write("a.txt", "one\n");
    repo.commit_all("base");
    let checkout = repo.checkout();
    checkout.save_looks_blocking(SESSION, &[look("a.txt", 1), look("b.txt", 2)]).expect("save");
    checkout.save_looks_blocking(SESSION, &[look("b.txt", 3)]).expect("replace");
    assert_eq!(checkout.looks_blocking(SESSION).expect("read"), [look("b.txt", 3)]);
    checkout.save_looks_blocking(SESSION, &[]).expect("clear");
    assert!(checkout.looks_blocking(SESSION).expect("read").is_empty());
    checkout.save_looks_blocking(SESSION, &[]).expect("clear again");
}

#[test]
fn a_session_id_unfit_for_a_ref_is_refused() {
    let repo = Repo::new();
    repo.write("a.txt", "one\n");
    repo.commit_all("base");
    assert!(repo.checkout().save_looks_blocking("../x", &[look("a", 1)]).is_err());
}
