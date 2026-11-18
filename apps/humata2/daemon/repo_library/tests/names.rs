//! Repo names as the library keeps them: `owner/name`, as GitHub allows.

use humata2_repo_library::{NameError, RepoName};

#[test]
fn owner_and_name_are_split_and_shown_as_given() {
    let repo = RepoName::parse("Bazelbuild/rules_java").unwrap();
    assert_eq!(repo.owner(), "Bazelbuild");
    assert_eq!(repo.name(), "rules_java");
    assert_eq!(repo.to_string(), "Bazelbuild/rules_java");
}

#[test]
fn names_differing_only_in_case_are_the_same_repo() {
    assert_eq!(
        RepoName::parse("ACME/Api").unwrap(),
        RepoName::parse("acme/api").unwrap()
    );
}

#[test]
fn a_github_url_or_a_dot_git_suffix_is_read_as_its_owner_and_name() {
    for text in [
        "https://github.com/acme/api",
        "https://github.com/acme/api.git",
        "git@github.com:acme/api.git",
        "acme/api.git",
        " acme/api ",
    ] {
        assert_eq!(
            RepoName::parse(text).unwrap(),
            RepoName::parse("acme/api").unwrap(),
            "{text}"
        );
    }
}

#[test]
fn text_that_is_not_owner_slash_name_is_refused() {
    for text in ["", "acme", "acme/", "/api", "acme/api/extra", "ac me/api", "-acme/api"] {
        assert!(
            matches!(RepoName::parse(text), Err(NameError { .. })),
            "{text:?} should be refused"
        );
    }
}
