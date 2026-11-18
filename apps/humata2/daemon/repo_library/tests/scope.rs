//! The default search scope in `config/repos.toml` (RL-7, RL-8, D474):
//! reading it, and writing it back without losing the user's comments or
//! other keys.

use humata2_repo_library::{Scope, ScopeError};

fn owners(list: &[&str], include_me: bool) -> Scope {
    Scope::Owners {
        owners: list.iter().map(|owner| (*owner).to_owned()).collect(),
        include_me,
    }
}

#[test]
fn no_file_or_no_owners_key_is_your_repos() {
    assert_eq!(Scope::read("").unwrap(), Scope::YourRepos);
    assert_eq!(
        Scope::read("# nothing set yet\ninclude_me = false\n").unwrap(),
        Scope::YourRepos
    );
}

#[test]
fn owners_are_read_in_order_and_include_me_defaults_to_on() {
    assert_eq!(
        Scope::read("owners = [\"bazelbuild\", \"buildfarm\"]\n").unwrap(),
        owners(&["bazelbuild", "buildfarm"], true)
    );
    assert_eq!(
        Scope::read("owners = [\"bazelbuild\"]\ninclude_me = false\n").unwrap(),
        owners(&["bazelbuild"], false)
    );
}

#[test]
fn an_owner_listed_twice_in_any_case_counts_once() {
    assert_eq!(
        Scope::read("owners = [\"bazelbuild\", \"BazelBuild\", \"buildbarn\"]").unwrap(),
        owners(&["bazelbuild", "buildbarn"], true)
    );
}

#[test]
fn a_file_that_is_not_toml_or_has_the_wrong_types_is_refused() {
    assert!(matches!(Scope::read("owners = ["), Err(ScopeError::Toml { .. })));
    assert!(matches!(
        Scope::read("owners = \"bazelbuild\""),
        Err(ScopeError::OwnersNotAList { .. })
    ));
    assert!(matches!(
        Scope::read("owners = [1]"),
        Err(ScopeError::OwnerNotText { .. })
    ));
    assert!(matches!(
        Scope::read("owners = [\"no spaces allowed\"]"),
        Err(ScopeError::BadOwner { .. })
    ));
    assert!(matches!(
        Scope::read("owners = []\ninclude_me = \"yes\""),
        Err(ScopeError::IncludeMeNotABool { .. })
    ));
}

#[test]
fn writing_a_scope_reads_back_as_the_same_scope() {
    for scope in [
        Scope::YourRepos,
        owners(&[], true),
        owners(&["bazelbuild"], false),
        owners(&["bazelbuild", "buildfarm", "bazel-contrib"], true),
    ] {
        let text = scope.write_into("").unwrap();
        assert_eq!(Scope::read(&text).unwrap(), scope, "{text}");
    }
}

#[test]
fn writing_keeps_comments_other_keys_and_tables() {
    let before = "# my usual orgs\nowners = [\"bazelbuild\"] # the main one\ntheme = \"dark\"\n\n[later]\nowners = \"not the scope\"\n";
    let after = owners(&["bazelbuild", "buildfarm"], true)
        .write_into(before)
        .unwrap();

    assert!(after.contains("# my usual orgs"), "{after}");
    assert!(after.contains("# the main one"), "{after}");
    assert!(after.contains("theme = \"dark\""), "{after}");
    assert!(after.contains("[later]\nowners = \"not the scope\""), "{after}");
    assert_eq!(
        Scope::read(&after).unwrap(),
        owners(&["bazelbuild", "buildfarm"], true)
    );
}

#[test]
fn writing_replaces_an_owners_list_that_spans_lines() {
    let before = "owners = [\n  \"bazelbuild\",\n  \"buildbarn\",\n]\ninclude_me = true\n";
    let after = owners(&["buildfarm"], false).write_into(before).unwrap();
    assert_eq!(Scope::read(&after).unwrap(), owners(&["buildfarm"], false));
    assert!(!after.contains("buildbarn"), "{after}");
}

#[test]
fn your_repos_is_written_by_taking_the_owners_key_out() {
    let after = Scope::YourRepos
        .write_into("# keep me\nowners = [\"bazelbuild\"]\n")
        .unwrap();
    assert!(after.contains("# keep me"), "{after}");
    assert_eq!(Scope::read(&after).unwrap(), Scope::YourRepos);
}

#[test]
fn all_of_github_is_for_one_search_and_is_never_written_as_the_default() {
    assert!(matches!(
        Scope::AllOfGitHub.write_into(""),
        Err(ScopeError::AllOfGitHubIsPerSearch)
    ));
}

#[test]
fn a_bad_owner_is_refused_when_writing_and_nothing_is_written() {
    assert!(matches!(
        owners(&["bad owner"], true).write_into("owners = [\"bazelbuild\"]\n"),
        Err(ScopeError::BadOwner { .. })
    ));
}
