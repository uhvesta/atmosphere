//! The base a session's changes are compared with, as the Diff panel picks
//! it: one preset for every repository, with per-repository overrides.

use humata2_clients::model::{DiffBase, DiffTargets, RepoBase, RepoDiffSpec, SessionId};

fn spec(repo: &str, base: RepoBase) -> RepoDiffSpec {
    RepoDiffSpec {
        repo: repo.to_owned(),
        base,
    }
}

fn names(repos: &[&str]) -> Vec<String> {
    repos.iter().map(|repo| (*repo).to_owned()).collect()
}

#[test]
fn an_empty_target_asks_for_the_preset() {
    let targets = DiffTargets {
        preset: DiffBase::SinceFork,
        ..DiffTargets::default()
    };
    assert_eq!(
        targets.base(&names(&["acme/api", "acme/web"])),
        DiffBase::SinceFork
    );
    assert!(!targets.per_repo());
}

#[test]
fn picking_one_repository_asks_for_a_spec_in_the_order_given() {
    let mut targets = DiffTargets::default();
    targets.pick("acme/api", RepoBase::Branch("release/2.4".to_owned()));
    assert!(targets.per_repo());
    assert_eq!(
        targets.base(&names(&["acme/web", "acme/api"])),
        DiffBase::Spec(vec![
            spec("acme/web", RepoBase::DefaultBranch),
            spec("acme/api", RepoBase::Branch("release/2.4".to_owned())),
        ])
    );
}

#[test]
fn a_since_fork_preset_keeps_fork_point_on_the_other_repository() {
    let mut targets = DiffTargets::default();
    targets.pick_all(DiffBase::SinceFork);
    targets.pick("acme/api", RepoBase::Branch("main".to_owned()));
    assert_eq!(targets.of("acme/web"), RepoBase::ForkPoint);
    assert_eq!(
        targets.base(&names(&["acme/api", "acme/web"])),
        DiffBase::Spec(vec![
            spec("acme/api", RepoBase::Branch("main".to_owned())),
            spec("acme/web", RepoBase::ForkPoint),
        ])
    );
}

#[test]
fn picking_under_another_session_resets_the_preset_to_the_default_branch() {
    let mut targets = DiffTargets::default();
    targets.pick_all(DiffBase::OtherSession(SessionId::new("brin")));
    targets.pick("acme/api", RepoBase::LastReviewed);
    assert_eq!(targets.of("acme/web"), RepoBase::DefaultBranch);
    assert_eq!(targets.of("acme/api"), RepoBase::LastReviewed);
}

#[test]
fn picking_all_drops_every_override() {
    let mut targets = DiffTargets::default();
    targets.pick("acme/api", RepoBase::Uncommitted);
    targets.pick("acme/web", RepoBase::LastReviewed);
    targets.pick_all(DiffBase::SinceLastReviewed);
    assert!(!targets.per_repo());
    assert_eq!(
        targets.base(&names(&["acme/api", "acme/web"])),
        DiffBase::SinceLastReviewed
    );
}

#[test]
fn a_target_survives_a_save_and_a_load() -> Result<(), serde_json::Error> {
    let mut targets = DiffTargets::default();
    targets.pick_all(DiffBase::OtherSession(SessionId::new("brin")));
    targets.pick("acme/api", RepoBase::Rev("v2.4.0".to_owned()));
    let saved = serde_json::to_string(&targets)?;
    let loaded: DiffTargets = serde_json::from_str(&saved)?;
    assert_eq!(loaded, targets);
    Ok(())
}

#[test]
fn an_empty_object_loads_as_the_default_target() -> Result<(), serde_json::Error> {
    let loaded: DiffTargets = serde_json::from_str("{}")?;
    assert_eq!(loaded, DiffTargets::default());
    Ok(())
}

#[test]
fn picking_one_repository_under_pull_request_bases_keeps_the_others_on_theirs() {
    let mut targets = DiffTargets::default();
    targets.pick_all(DiffBase::PullRequestBase);
    targets.pick("acme/api", RepoBase::Uncommitted);
    assert_eq!(targets.preset, DiffBase::PullRequestBase);
    assert_eq!(
        targets.base(&names(&["acme/api", "acme/web"])),
        DiffBase::Spec(vec![
            spec("acme/api", RepoBase::Uncommitted),
            spec("acme/web", RepoBase::PullRequestBase),
        ])
    );
}
