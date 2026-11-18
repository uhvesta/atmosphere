//! One repo search (RL-6, RL-9, D274, D476, D477): which GitHub search a
//! query and scope make, and how the library and GitHub groups are merged.

use humata2_repo_library::{
    Entry, GitHubQuery, Hit, Place, RepoName, Scope, github_query, merge,
};

fn name(full: &str) -> RepoName {
    RepoName::parse(full).unwrap()
}

/// Library entries in list order (most recently used first).
fn library(names: &[&str]) -> Vec<Entry> {
    let count = i64::try_from(names.len()).unwrap();
    names
        .iter()
        .zip((0..count).rev())
        .map(|(full, at)| Entry {
            repo: name(full),
            remote: format!("https://github.com/{full}.git"),
            default_branch: "main".to_owned(),
            added_at: at,
            last_used_at: at,
            last_used_in: Place::Desk,
        })
        .collect()
}

fn hits(names: &[&str]) -> Vec<Hit> {
    names
        .iter()
        .map(|full| Hit {
            repo: name(full),
            description: String::new(),
            is_private: false,
        })
        .collect()
}

fn owners(list: &[&str], include_me: bool) -> Scope {
    Scope::Owners {
        owners: list.iter().map(|owner| (*owner).to_owned()).collect(),
        include_me,
    }
}

#[test]
fn library_matches_come_whole_ones_first_then_most_recently_used() {
    let lib = library(&["acme/docs", "octo/notes", "acme/ledger", "acme/notes-old"]);
    let found = merge("notes", &lib, &[]);
    let shown: Vec<String> = found.library.iter().map(|e| e.repo.to_string()).collect();
    assert_eq!(shown, ["octo/notes", "acme/notes-old"]);

    let found = merge("es", &lib, &[]);
    let shown: Vec<String> = found.library.iter().map(|e| e.repo.to_string()).collect();
    assert_eq!(shown, ["octo/notes", "acme/notes-old", "acme/docs"]);
}

#[test]
fn an_empty_query_shows_the_whole_library_in_its_order() {
    let lib = library(&["acme/web", "acme/api"]);
    let found = merge("", &lib, &[]);
    let shown: Vec<String> = found.library.iter().map(|e| e.repo.to_string()).collect();
    assert_eq!(shown, ["acme/web", "acme/api"]);
}

#[test]
fn github_hits_already_shown_under_library_are_left_out_and_the_rest_keep_githubs_order() {
    let lib = library(&["bazelbuild/rules_java"]);
    let found = merge(
        "rules_j",
        &lib,
        &hits(&["bazelbuild/rules_jsonnet", "BazelBuild/Rules_Java", "bazelbuild/rules_jvm"]),
    );
    let github: Vec<String> = found.github.iter().map(|h| h.repo.to_string()).collect();
    assert_eq!(github, ["bazelbuild/rules_jsonnet", "bazelbuild/rules_jvm"]);
}

#[test]
fn a_library_repo_the_query_does_not_match_does_not_hide_its_github_hit() {
    let lib = library(&["acme/api"]);
    let found = merge("web", &lib, &hits(&["acme/api", "acme/web"]));
    assert!(found.library.is_empty());
    assert_eq!(found.github.len(), 2);
}

#[test]
fn owners_make_one_search_qualified_by_each_owner_and_by_me() {
    assert_eq!(
        github_query("rules java", &owners(&["bazelbuild", "buildfarm"], true), Some("uh")),
        GitHubQuery::Search("rules java user:bazelbuild user:buildfarm user:uh".to_owned())
    );
    assert_eq!(
        github_query("rules", &owners(&["bazelbuild"], false), Some("uh")),
        GitHubQuery::Search("rules user:bazelbuild".to_owned())
    );
    assert_eq!(
        github_query("rules", &owners(&["bazelbuild"], true), None),
        GitHubQuery::Search("rules user:bazelbuild".to_owned()),
        "with no login known, the owners still search"
    );
}

#[test]
fn qualifiers_typed_in_the_query_cannot_widen_the_scope() {
    assert_eq!(
        github_query("rules org:evil user:other", &owners(&["bazelbuild"], false), None),
        GitHubQuery::Search("rules user:bazelbuild".to_owned())
    );
}

#[test]
fn a_scoped_search_needs_two_characters_and_all_of_github_needs_three() {
    let scoped = owners(&["bazelbuild"], false);
    assert_eq!(github_query("r", &scoped, None), GitHubQuery::TooShort { needs: 2 });
    assert!(matches!(github_query("rb", &scoped, None), GitHubQuery::Search(_)));

    assert_eq!(
        github_query("rb", &Scope::AllOfGitHub, None),
        GitHubQuery::TooShort { needs: 3 }
    );
    assert_eq!(
        github_query(" rbe ", &Scope::AllOfGitHub, None),
        GitHubQuery::Search("rbe".to_owned())
    );
}

#[test]
fn your_repos_is_the_listing_matched_locally_at_any_length() {
    assert_eq!(github_query("", &Scope::YourRepos, Some("uh")), GitHubQuery::Listing);
    assert_eq!(github_query("a", &Scope::YourRepos, None), GitHubQuery::Listing);
}

#[test]
fn a_scope_with_no_owners_and_not_me_searches_nothing() {
    assert_eq!(github_query("rules", &owners(&[], false), Some("uh")), GitHubQuery::Nothing);
    assert_eq!(
        github_query("rules", &owners(&[], true), None),
        GitHubQuery::Nothing,
        "only me, but no login known"
    );
    assert_eq!(
        github_query("rules", &owners(&[], true), Some("uh")),
        GitHubQuery::Search("rules user:uh".to_owned())
    );
}
