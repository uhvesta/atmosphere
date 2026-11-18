//! The repo library on the host (RL-1, RL-2, RL-4): what a use records,
//! the order it lists in, Remove and Forget all, and what is still there
//! after the file is closed and opened again.

use humata2_repo_library::{Library, LibraryError, Place, RepoName, Use};

fn name(full: &str) -> RepoName {
    RepoName::parse(full).unwrap()
}

fn used(full: &str, at: i64, place: Place) -> Use {
    Use {
        repo: name(full),
        remote: format!("https://github.com/{full}.git"),
        default_branch: "main".to_owned(),
        at,
        place,
        canonical: false,
    }
}

fn channel(id: &str) -> Place {
    Place::Channel(id.to_owned())
}

fn listed(library: &Library) -> Vec<String> {
    let (entries, _) = library.list().unwrap();
    entries.iter().map(|entry| entry.repo.to_string()).collect()
}

fn open(dir: &tempfile::TempDir) -> Library {
    Library::open(&dir.path().join("repos.db")).unwrap()
}

#[test]
fn a_new_library_is_empty_at_revision_zero() {
    let dir = tempfile::tempdir().unwrap();
    let (entries, revision) = open(&dir).list().unwrap();
    assert!(entries.is_empty());
    assert_eq!(revision, 0);
}

#[test]
fn a_recorded_use_keeps_the_repo_its_remote_branch_and_when_and_where() {
    let dir = tempfile::tempdir().unwrap();
    let library = open(&dir);
    library.record(&used("acme/api", 1_000, channel("ch_1"))).unwrap();

    let (entries, _) = library.list().unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.repo, name("acme/api"));
    assert_eq!(entry.remote, "https://github.com/acme/api.git");
    assert_eq!(entry.default_branch, "main");
    assert_eq!(entry.added_at, 1_000);
    assert_eq!(entry.last_used_at, 1_000);
    assert_eq!(entry.last_used_in, channel("ch_1"));
}

#[test]
fn using_a_repo_again_moves_last_used_forward_and_keeps_when_it_was_added() {
    let dir = tempfile::tempdir().unwrap();
    let library = open(&dir);
    library.record(&used("acme/api", 1_000, channel("ch_1"))).unwrap();
    library.record(&used("acme/api", 5_000, Place::Desk)).unwrap();

    let (entries, _) = library.list().unwrap();
    assert_eq!(entries.len(), 1, "one repo is one entry");
    assert_eq!(entries[0].added_at, 1_000);
    assert_eq!(entries[0].last_used_at, 5_000);
    assert_eq!(entries[0].last_used_in, Place::Desk);
}

#[test]
fn an_older_use_arriving_late_does_not_move_last_used_back() {
    let dir = tempfile::tempdir().unwrap();
    let library = open(&dir);
    library.record(&used("acme/api", 5_000, Place::Desk)).unwrap();
    library.record(&used("acme/api", 1_000, channel("ch_1"))).unwrap();

    let (entries, _) = library.list().unwrap();
    assert_eq!(entries[0].last_used_at, 5_000);
    assert_eq!(entries[0].last_used_in, Place::Desk);
}

#[test]
fn owner_and_name_are_one_repo_whatever_their_case() {
    let dir = tempfile::tempdir().unwrap();
    let library = open(&dir);
    library.record(&used("Acme/API", 1_000, Place::Desk)).unwrap();
    library.record(&used("acme/api", 2_000, Place::Desk)).unwrap();
    assert_eq!(library.list().unwrap().0.len(), 1);
}

#[test]
fn only_github_s_own_spelling_changes_the_stored_casing() {
    let dir = tempfile::tempdir().unwrap();
    let library = open(&dir);
    library.record(&used("acme/api", 1_000, Place::Desk)).unwrap();
    let typed = library.record(&used("ACME/Api", 2_000, Place::Desk)).unwrap();
    assert_eq!(listed(&library), ["acme/api"]);

    let canonical = Use {
        canonical: true,
        ..used("Acme/API", 1_500, Place::Desk)
    };
    let renamed = library.record(&canonical).unwrap();
    assert_eq!(listed(&library), ["Acme/API"]);
    assert!(renamed > typed);

    let again = library.record(&canonical).unwrap();
    assert_eq!(again, renamed);
    let (entries, _) = library.list().unwrap();
    assert_eq!(entries[0].last_used_at, 2_000);
}

#[test]
fn the_library_lists_the_most_recently_used_first() {
    let dir = tempfile::tempdir().unwrap();
    let library = open(&dir);
    library.record(&used("acme/api", 1_000, Place::Desk)).unwrap();
    library.record(&used("acme/ledger", 3_000, Place::Desk)).unwrap();
    library.record(&used("acme/web", 2_000, Place::Desk)).unwrap();
    assert_eq!(listed(&library), ["acme/ledger", "acme/web", "acme/api"]);

    library.record(&used("acme/api", 4_000, Place::Desk)).unwrap();
    assert_eq!(listed(&library), ["acme/api", "acme/ledger", "acme/web"]);
}

#[test]
fn remove_takes_out_one_repo_and_says_whether_it_was_there() {
    let dir = tempfile::tempdir().unwrap();
    let library = open(&dir);
    library.record(&used("acme/api", 1_000, Place::Desk)).unwrap();
    library.record(&used("acme/web", 2_000, Place::Desk)).unwrap();

    assert!(library.remove(&name("ACME/api")).unwrap());
    assert_eq!(listed(&library), ["acme/web"]);
    assert!(!library.remove(&name("acme/api")).unwrap());
}

#[test]
fn a_removed_repo_comes_back_as_new_when_it_is_used_again() {
    let dir = tempfile::tempdir().unwrap();
    let library = open(&dir);
    library.record(&used("acme/api", 1_000, Place::Desk)).unwrap();
    library.remove(&name("acme/api")).unwrap();
    library.record(&used("acme/api", 9_000, Place::Desk)).unwrap();

    let (entries, _) = library.list().unwrap();
    assert_eq!(entries[0].added_at, 9_000);
}

#[test]
fn forget_all_empties_the_library_and_counts_what_it_forgot() {
    let dir = tempfile::tempdir().unwrap();
    let library = open(&dir);
    library.record(&used("acme/api", 1_000, Place::Desk)).unwrap();
    library.record(&used("acme/web", 2_000, Place::Desk)).unwrap();

    assert_eq!(library.forget_all().unwrap(), 2);
    assert!(listed(&library).is_empty());
    assert_eq!(library.forget_all().unwrap(), 0);
}

#[test]
fn every_change_raises_the_revision_and_a_no_op_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let library = open(&dir);
    let revision = |library: &Library| library.list().unwrap().1;

    let mut last = revision(&library);
    library.record(&used("acme/api", 1_000, Place::Desk)).unwrap();
    assert!(revision(&library) > last, "a new repo");
    last = revision(&library);
    library.record(&used("acme/api", 2_000, Place::Desk)).unwrap();
    assert!(revision(&library) > last, "a later use");
    last = revision(&library);
    library.remove(&name("acme/api")).unwrap();
    assert!(revision(&library) > last, "a remove");
    last = revision(&library);

    library.remove(&name("acme/api")).unwrap();
    library.forget_all().unwrap();
    assert_eq!(revision(&library), last, "nothing there to remove or forget");
}

#[test]
fn the_library_and_its_revision_survive_closing_and_opening_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let revision = {
        let library = open(&dir);
        library.record(&used("acme/api", 1_000, channel("ch_1"))).unwrap();
        library.record(&used("acme/web", 2_000, Place::Desk)).unwrap();
        library.list().unwrap().1
    };
    let library = open(&dir);
    let (entries, again) = library.list().unwrap();
    assert_eq!(again, revision);
    assert_eq!(listed(&library), ["acme/web", "acme/api"]);
    assert_eq!(entries[1].last_used_in, channel("ch_1"));
}

#[test]
fn a_file_that_is_not_a_library_is_refused_and_left_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("repos.db");
    std::fs::write(&path, b"not a database at all, just some text").unwrap();

    let refused = Library::open(&path);
    assert!(matches!(refused, Err(LibraryError::Database { .. })));
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"not a database at all, just some text"
    );
}
