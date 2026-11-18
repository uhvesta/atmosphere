//! A stack of layers (RV-36, D800): each layer's own diff in every repo it
//! touches, Whole stack as the union of the layers, the refusals that say
//! which layer and repo to fix, and drift once a kept rev moves.

mod support;

use std::collections::BTreeMap;

use humata2_review_diff::{
    Clones, DiffSpec, DriftKind, Layer, LayerRepo, Part, RepoDiff, Stack, StackError,
};
use support::{Repo, context, paths};

/// Two repos and the milestone's three layers:
/// - api: `main` (a.txt), `codec` (+src/codec.rs), `journal` on codec
///   (+src/journal/log.rs, codec.rs edited), checked out at journal with
///   uncommitted src/journal/wip.rs (untracked), an edit of
///   src/journal/log.rs, and a stray untracked other/stray.txt.
/// - web: `main` (index.html), `codec` (+codec.js).
struct Fixture {
    api: Repo,
    web: Repo,
}

fn fixture() -> Fixture {
    let api = Repo::new();
    api.write("a.txt", "trunk\n");
    api.commit_all("trunk");
    api.git(&["checkout", "-q", "-b", "codec"]);
    api.write("src/codec.rs", "fn encode() {}\n");
    api.commit_all("codec");
    api.git(&["checkout", "-q", "-b", "journal"]);
    api.write("src/journal/log.rs", "fn append() {}\n");
    api.write("src/codec.rs", "fn encode() {}\nfn decode() {}\n");
    api.commit_all("journal");
    api.write("src/journal/log.rs", "fn append() {}\nfn flush() {}\n");
    api.write("src/journal/wip.rs", "// soon\n");
    api.write("other/stray.txt", "not in any layer\n");

    let web = Repo::new();
    web.write("index.html", "<p>hi</p>\n");
    web.commit_all("trunk");
    web.git(&["checkout", "-q", "-b", "codec"]);
    web.write("codec.js", "export {}\n");
    web.commit_all("codec");
    Fixture { api, web }
}

fn clones(fixture: &Fixture) -> Clones {
    Clones::new(BTreeMap::from([
        ("api".to_owned(), fixture.api.checkout()),
        ("web".to_owned(), fixture.web.checkout()),
    ]))
}

fn range(repo: &str, base: &str, head: &str) -> LayerRepo {
    LayerRepo {
        repo: repo.to_owned(),
        part: Part::Range {
            base: base.to_owned(),
            head: head.to_owned(),
        },
        pull_request: None,
    }
}

fn worktree(repo: &str, paths: &[&str]) -> LayerRepo {
    LayerRepo {
        repo: repo.to_owned(),
        part: Part::Worktree {
            include_paths: paths.iter().map(|path| (*path).to_owned()).collect(),
        },
        pull_request: None,
    }
}

fn layer(title: &str, repos: Vec<LayerRepo>) -> Layer {
    Layer {
        title: title.to_owned(),
        purpose: None,
        repos,
    }
}

/// The milestone's example, bottom first.
fn three_layers() -> Stack {
    Stack::new(vec![
        layer(
            "codec",
            vec![range("api", "main", "codec"), range("web", "main", "codec")],
        ),
        layer("journal", vec![range("api", "codec", "journal")]),
        layer("Not committed yet", vec![worktree("api", &["src/journal/"])]),
    ])
}

/// Each repo of `spec` diffed, keyed by repo.
fn diffs(fixture: &Fixture, spec: &DiffSpec) -> BTreeMap<String, RepoDiff> {
    let clones = clones(fixture);
    spec.repos
        .iter()
        .map(|repo| {
            let checkout = clones.get(&repo.repo).expect("a clone of the repo");
            let diff = checkout
                .diff_blocking(repo, &context())
                .expect("compute the layer's diff");
            (repo.repo.clone(), diff)
        })
        .collect()
}

fn shown(fixture: &Fixture, spec: &DiffSpec) -> BTreeMap<String, Vec<String>> {
    diffs(fixture, spec)
        .iter()
        .map(|(repo, diff)| (repo.clone(), paths(diff)))
        .collect()
}

fn expect(pairs: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
    pairs
        .iter()
        .map(|(repo, paths)| {
            (
                (*repo).to_owned(),
                paths.iter().map(|path| (*path).to_owned()).collect(),
            )
        })
        .collect()
}

#[test]
fn each_layer_shows_its_own_diff_in_only_the_repos_it_touches() {
    let fixture = fixture();
    let checked = three_layers()
        .check_blocking(&clones(&fixture))
        .expect("the stack is valid");
    let codec = checked.layer_spec("codec").expect("layer 1");
    assert_eq!(
        shown(&fixture, &codec),
        expect(&[("api", &["src/codec.rs"]), ("web", &["codec.js"])])
    );
    let journal = checked.layer_spec("journal").expect("layer 2");
    assert_eq!(
        shown(&fixture, &journal),
        expect(&[("api", &["src/codec.rs", "src/journal/log.rs"])])
    );
    let wip = checked.layer_spec("Not committed yet").expect("layer 3");
    assert_eq!(
        shown(&fixture, &wip),
        expect(&[("api", &["src/journal/log.rs", "src/journal/wip.rs"])])
    );
}

#[test]
fn a_layer_is_found_by_its_title_ignoring_case_and_spaces() {
    let fixture = fixture();
    let checked = three_layers()
        .check_blocking(&clones(&fixture))
        .expect("the stack is valid");
    assert_eq!(
        checked.layer_spec("  Journal ").ok(),
        checked.layer_spec("journal").ok()
    );
    assert!(checked.layer_spec("journal").is_ok());
    let error = checked.layer_spec("nope").expect_err("no such layer");
    assert!(
        matches!(&error, StackError::UnknownLayer { known, .. } if known.len() == 3),
        "{error:?}"
    );
}

#[test]
fn whole_stack_is_the_union_of_the_layers_and_leaves_out_stray_work() {
    let fixture = fixture();
    let checked = three_layers()
        .check_blocking(&clones(&fixture))
        .expect("the stack is valid");
    // other/stray.txt is uncommitted but in no layer, so not in Whole stack.
    assert_eq!(
        shown(&fixture, &checked.whole()),
        expect(&[
            (
                "api",
                &["src/codec.rs", "src/journal/log.rs", "src/journal/wip.rs"]
            ),
            ("web", &["codec.js"]),
        ])
    );
}

#[test]
fn a_range_layer_is_the_same_wherever_the_clone_is_checked_out() {
    let fixture = fixture();
    let ranges = Stack::new(vec![
        layer("codec", vec![range("api", "main", "codec")]),
        layer("journal", vec![range("api", "codec", "journal")]),
    ]);
    let at_top = ranges
        .check_blocking(&clones(&fixture))
        .expect("the stack is valid");
    let top = shown(&fixture, &at_top.layer_spec("journal").expect("layer 2"));
    fixture.api.git(&["stash", "-u", "-q"]);
    fixture.api.git(&["checkout", "-q", "main"]);
    let at_trunk = ranges
        .check_blocking(&clones(&fixture))
        .expect("still valid on the trunk");
    let trunk = shown(&fixture, &at_trunk.layer_spec("journal").expect("layer 2"));
    assert_eq!(top, trunk);
    assert_eq!(top, expect(&[("api", &["src/codec.rs", "src/journal/log.rs"])]));
}

#[test]
fn a_worktree_layer_with_no_work_under_its_paths_is_accepted_and_empty() {
    let fixture = fixture();
    let stack = Stack::new(vec![layer("docs", vec![worktree("api", &["docs/"])])]);
    let checked = stack
        .check_blocking(&clones(&fixture))
        .expect("an empty worktree layer is fine");
    let spec = checked.layer_spec("docs").expect("the layer");
    assert_eq!(shown(&fixture, &spec), expect(&[("api", &[])]));
}

#[test]
fn checking_resolves_each_rev_to_its_commit() {
    let fixture = fixture();
    let checked = three_layers()
        .check_blocking(&clones(&fixture))
        .expect("the stack is valid");
    let resolved = checked.resolved("journal", "api").expect("resolved");
    assert_eq!(resolved.base_commit, fixture.api.git(&["rev-parse", "codec"]));
    assert_eq!(resolved.head_commit, fixture.api.git(&["rev-parse", "journal"]));
}

/// The stack check's refusal for `stack` on the fixture.
fn refusal(stack: Stack) -> StackError {
    let fixture = fixture();
    stack
        .check_blocking(&clones(&fixture))
        .expect_err("the stack is refused")
}

#[test]
fn a_stack_with_no_layers_or_too_many_is_refused() {
    assert!(matches!(refusal(Stack::new(vec![])), StackError::NoLayers));
    let many = (0..51)
        .map(|n| layer(&format!("l{n}"), vec![worktree("api", &["src/"])]))
        .collect();
    assert!(matches!(
        refusal(Stack::new(many)),
        StackError::TooManyLayers { count: 51, max: 50 }
    ));
}

#[test]
fn an_empty_layer_or_a_repo_twice_in_one_layer_is_refused_by_name() {
    let empty = refusal(Stack::new(vec![layer("codec", vec![])]));
    assert!(
        matches!(&empty, StackError::EmptyLayer { layer } if layer == "codec"),
        "{empty:?}"
    );
    let twice = refusal(Stack::new(vec![layer(
        "codec",
        vec![range("api", "main", "codec"), range("api", "main", "codec")],
    )]));
    assert!(
        matches!(&twice, StackError::RepoTwice { layer, repo } if layer == "codec" && repo == "api"),
        "{twice:?}"
    );
}

#[test]
fn titles_must_be_short_present_and_unique_ignoring_case() {
    let blank = refusal(Stack::new(vec![layer("  ", vec![range("api", "main", "codec")])]));
    assert!(matches!(blank, StackError::BadTitle { .. }), "{blank:?}");
    let long = refusal(Stack::new(vec![layer(
        &"x".repeat(61),
        vec![range("api", "main", "codec")],
    )]));
    assert!(matches!(long, StackError::BadTitle { .. }), "{long:?}");
    let same = refusal(Stack::new(vec![
        layer("codec", vec![range("api", "main", "codec")]),
        layer("Codec", vec![range("api", "codec", "journal")]),
    ]));
    assert!(matches!(same, StackError::DuplicateTitle { .. }), "{same:?}");
}

#[test]
fn a_purpose_is_one_short_line() {
    for purpose in ["two\nlines", &"p".repeat(121)] {
        let mut codec = layer("codec", vec![range("api", "main", "codec")]);
        codec.purpose = Some(purpose.to_owned());
        let error = refusal(Stack::new(vec![codec]));
        assert!(
            matches!(&error, StackError::BadPurpose { layer, .. } if layer == "codec"),
            "{error:?}"
        );
    }
}

#[test]
fn a_repo_the_session_does_not_have_is_refused_by_layer() {
    let error = refusal(Stack::new(vec![layer(
        "codec",
        vec![range("mobile", "main", "codec")],
    )]));
    assert!(
        matches!(&error, StackError::UnknownRepo { layer, repo, .. } if layer == "codec" && repo == "mobile"),
        "{error:?}"
    );
}

#[test]
fn a_rev_that_does_not_resolve_is_refused_by_layer_and_repo() {
    let error = refusal(Stack::new(vec![layer(
        "codec",
        vec![range("api", "main", "no-such-branch")],
    )]));
    assert!(
        matches!(&error, StackError::UnknownRevision { layer, repo, rev } if layer == "codec" && repo == "api" && rev == "no-such-branch"),
        "{error:?}"
    );
}

#[test]
fn a_head_that_is_not_a_descendant_of_its_base_is_refused() {
    let error = refusal(Stack::new(vec![layer(
        "backwards",
        vec![range("api", "journal", "main")],
    )]));
    assert!(
        matches!(&error, StackError::NotAncestor { layer, repo, .. } if layer == "backwards" && repo == "api"),
        "{error:?}"
    );
}

#[test]
fn layers_that_do_not_chain_in_a_repo_are_a_fork_and_refused() {
    // journal's base should be codec's head; main leaves codec out.
    let error = refusal(Stack::new(vec![
        layer("codec", vec![range("api", "main", "codec")]),
        layer("journal", vec![range("api", "main", "journal")]),
    ]));
    assert!(
        matches!(&error, StackError::ChainBreak { layer, repo, .. } if layer == "journal" && repo == "api"),
        "{error:?}"
    );
}

#[test]
fn a_worktree_layer_must_sit_above_the_ranges_on_the_top_head() {
    let below = refusal(Stack::new(vec![
        layer("wip", vec![worktree("api", &["src/"])]),
        layer("codec", vec![range("api", "main", "codec")]),
    ]));
    assert!(
        matches!(&below, StackError::WorktreeBelowRange { layer, repo } if layer == "wip" && repo == "api"),
        "{below:?}"
    );
    // HEAD is journal, not the top range head codec.
    let off_top = refusal(Stack::new(vec![
        layer("codec", vec![range("api", "main", "codec")]),
        layer("wip", vec![worktree("api", &["src/"])]),
    ]));
    assert!(
        matches!(&off_top, StackError::HeadNotAtTop { layer, repo, .. } if layer == "wip" && repo == "api"),
        "{off_top:?}"
    );
    let no_paths = refusal(Stack::new(vec![layer("wip", vec![worktree("api", &[])])]));
    assert!(
        matches!(&no_paths, StackError::NoIncludePaths { layer, repo } if layer == "wip" && repo == "api"),
        "{no_paths:?}"
    );
}

#[test]
fn a_stack_that_still_holds_has_no_drift() {
    let fixture = fixture();
    let checked = three_layers()
        .check_blocking(&clones(&fixture))
        .expect("the stack is valid");
    assert!(checked.drift_blocking(&clones(&fixture)).is_empty());
}

#[test]
fn an_amended_lower_layer_drifts_the_layer_above_it() {
    let fixture = fixture();
    let checked = three_layers()
        .check_blocking(&clones(&fixture))
        .expect("the stack is valid");
    // codec is amended; journal still sits on the old codec commit.
    fixture.api.git(&["stash", "-u", "-q"]);
    fixture.api.git(&["checkout", "-q", "codec"]);
    fixture.api.write("src/codec.rs", "fn encode() { todo!() }\n");
    fixture.api.git(&["commit", "-q", "-a", "--amend", "-m", "codec"]);
    fixture.api.git(&["checkout", "-q", "journal"]);
    fixture.api.git(&["stash", "pop", "-q"]);
    let drift = checked.drift_blocking(&clones(&fixture));
    assert!(
        drift
            .iter()
            .any(|d| d.layer == "journal" && d.repo == "api" && d.kind == DriftKind::Moved),
        "{drift:?}"
    );
    assert!(drift.iter().all(|d| d.repo == "api"), "web is untouched: {drift:?}");
}

#[test]
fn a_rev_that_is_gone_drifts_as_missing() {
    let fixture = fixture();
    let checked = three_layers()
        .check_blocking(&clones(&fixture))
        .expect("the stack is valid");
    fixture.web.git(&["checkout", "-q", "main"]);
    fixture.web.git(&["branch", "-q", "-D", "codec"]);
    let drift = checked.drift_blocking(&clones(&fixture));
    assert!(
        drift
            .iter()
            .any(|d| d.layer == "codec" && d.repo == "web" && d.kind == DriftKind::Missing),
        "{drift:?}"
    );
}

#[test]
fn head_leaving_the_top_drifts_the_worktree_layer() {
    let fixture = fixture();
    let checked = three_layers()
        .check_blocking(&clones(&fixture))
        .expect("the stack is valid");
    fixture.api.git(&["stash", "-u", "-q"]);
    fixture.api.git(&["checkout", "-q", "codec"]);
    let drift = checked.drift_blocking(&clones(&fixture));
    assert!(
        drift.iter().any(|d| d.layer == "Not committed yet"
            && d.repo == "api"
            && d.kind == DriftKind::HeadMoved),
        "{drift:?}"
    );
}
