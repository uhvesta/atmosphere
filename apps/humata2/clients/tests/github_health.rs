//! A machine's GitHub row: "not found" and "not signed in" are told apart,
//! since each has its own fix (owner, 0.2.1).

use humata2_clients::machines::{GhLogin, GitHubState, MachineHealth};

fn health(gh: Option<GhLogin>, gh_version: &str) -> MachineHealth {
    MachineHealth {
        gh,
        agents: Vec::new(),
        disk_free: 0,
        disk_total: 0,
        variables: Vec::new(),
        skills_problem: None,
        git_version: String::new(),
        gh_version: gh_version.to_owned(),
    }
}

#[test]
fn gh_that_never_ran_is_not_found() {
    assert_eq!(health(None, "").github(), GitHubState::NotFound);
}

#[test]
fn gh_that_runs_without_an_account_is_signed_out() {
    assert_eq!(health(None, "2.83.0").github(), GitHubState::SignedOut);
}

#[test]
fn gh_with_an_account_is_signed_in() {
    let login = GhLogin {
        user: "octo".to_owned(),
        scopes: vec!["repo".to_owned()],
    };
    let health = health(Some(login.clone()), "2.83.0");
    assert_eq!(health.github(), GitHubState::SignedIn(&login));
}
