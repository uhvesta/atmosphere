//! File importance (D802): the daemon's own skims, the agent's tiers over
//! them, the arrangement with key and skim files, and the order and
//! importance a picked layer or the whole stack reads by.

mod support;

use std::collections::{BTreeMap, HashMap, HashSet};

use humata2_review_diff::{
    Clones, FileRef, FileStatus, FileSummary, Group, Importance, ImportanceError, Layer,
    LayerRepo, Part, RepoDiff, RepoSpec, ReviewPlan, Stack, Tier, Trees, check_importance,
    final_tier, tiers_blocking, tiers_of,
};
use support::{Repo, diff};

fn file(repo: &str, path: &str) -> FileRef {
    FileRef {
        repo: repo.to_owned(),
        path: path.to_owned(),
    }
}

fn mark(repo: &str, path: &str, tier: Tier) -> Importance {
    Importance {
        file: file(repo, path),
        tier,
    }
}

fn summary(path: &str, status: FileStatus) -> FileSummary {
    FileSummary {
        path: path.to_owned(),
        status,
        added: 1,
        removed: 0,
        binary: false,
        old_blob: None,
        new_blob: Some(format!("blob-{path}")),
        old_mode: 0,
        new_mode: 100_644,
    }
}

fn fake_diff(repo: &str, paths: &[&str]) -> RepoDiff {
    RepoDiff {
        repo: repo.to_owned(),
        branch: None,
        head: None,
        label: String::new(),
        trees: Trees {
            old: String::new(),
            new: String::new(),
        },
        files: paths
            .iter()
            .map(|path| summary(path, FileStatus::Modified))
            .collect(),
    }
}

fn names(files: &[FileRef]) -> Vec<&str> {
    files.iter().map(|file| file.path.as_str()).collect()
}

fn layout(plan: &ReviewPlan, diffs: &[RepoDiff], tiers: &[(&str, &str, Tier)]) -> Vec<(Option<String>, bool, Vec<String>)> {
    let tiers: HashMap<FileRef, Tier> = tiers
        .iter()
        .map(|(repo, path, tier)| (file(repo, path), *tier))
        .collect();
    plan.arrange_with(diffs, &tiers)
        .sections
        .into_iter()
        .map(|section| {
            (
                section.title,
                section.skim,
                section.files.into_iter().map(|file| file.path).collect(),
            )
        })
        .collect()
}

fn strings(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|path| (*path).to_owned()).collect()
}

// -- the daemon's own skims ------------------------------------------------

#[test]
fn lockfiles_by_name_in_any_folder_and_generated_paths_are_skimmed() {
    let repo = Repo::new();
    repo.write(".gitattributes", "gen/** linguist-generated\nkeep.rs -linguist-generated\n");
    for path in ["a.rs", "Cargo.lock", "web/yarn.lock", "gen/api.rs", "keep.rs"] {
        repo.write(path, "one\n");
    }
    repo.commit_all("base");
    for path in ["a.rs", "Cargo.lock", "web/yarn.lock", "gen/api.rs", "keep.rs"] {
        repo.write(path, "two\n");
    }
    let spec = RepoSpec::uncommitted("api");
    let shown = diff(&repo, &spec);
    let tiers = tiers_blocking(&repo.checkout(), &shown, &[]).expect("tiers");
    let expected: HashMap<String, Tier> = ["Cargo.lock", "web/yarn.lock", "gen/api.rs"]
        .iter()
        .map(|path| ((*path).to_owned(), Tier::Skim))
        .collect();
    assert_eq!(tiers, expected);
}

#[test]
fn a_rename_with_no_content_change_is_skimmed_but_an_edited_one_is_not() {
    let repo = Repo::new();
    repo.write("old_name.rs", "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\n");
    repo.write("moved_and_edited.rs", "fn x() {}\nfn y() {}\nfn z() {}\nfn w() {}\n");
    repo.commit_all("base");
    repo.git(&["checkout", "-q", "-b", "work"]);
    repo.git(&["mv", "old_name.rs", "new_name.rs"]);
    repo.git(&["mv", "moved_and_edited.rs", "elsewhere.rs"]);
    repo.write("elsewhere.rs", "fn x() {}\nfn y() {}\nfn z() {}\nfn w2() {}\n");
    repo.commit_all("move");
    let shown = diff(&repo, &RepoSpec::since_branch("api", "main"));
    let tiers = tiers_blocking(&repo.checkout(), &shown, &[]).expect("tiers");
    assert_eq!(tiers, HashMap::from([("new_name.rs".to_owned(), Tier::Skim)]));
}

#[test]
fn a_rename_with_equal_blobs_is_skimmed_without_asking_git() {
    let mut moved = summary(
        "b.rs",
        FileStatus::Renamed {
            from: "a.rs".to_owned(),
            similarity: 100,
        },
    );
    moved.old_blob = moved.new_blob.clone();
    moved.added = 0;
    let mut diff = fake_diff("api", &[]);
    diff.files.push(moved);
    assert_eq!(
        tiers_of(&diff, &HashSet::new(), &[]),
        HashMap::from([("b.rs".to_owned(), Tier::Skim)])
    );
}

// -- the final tier --------------------------------------------------------

#[test]
fn the_agents_tier_wins_over_the_daemons_skim_either_way() {
    let shown = fake_diff("api", &["Cargo.lock", "yarn.lock", "go.sum", "a.rs", "b.rs"]);
    let agent = [
        mark("api", "Cargo.lock", Tier::Normal),
        mark("api", "yarn.lock", Tier::Key),
        mark("api", "a.rs", Tier::Skim),
        mark("api", "b.rs", Tier::Key),
        mark("web", "go.sum", Tier::Key),
    ];
    let tiers = tiers_of(&shown, &HashSet::new(), &agent);
    assert_eq!(
        tiers,
        HashMap::from([
            ("yarn.lock".to_owned(), Tier::Key),
            ("go.sum".to_owned(), Tier::Skim),
            ("a.rs".to_owned(), Tier::Skim),
            ("b.rs".to_owned(), Tier::Key),
        ])
    );
}

#[test]
fn the_final_tier_rule() {
    assert_eq!(final_tier(None, false), Tier::Normal);
    assert_eq!(final_tier(None, true), Tier::Skim);
    assert_eq!(final_tier(Some(Tier::Normal), true), Tier::Normal);
    assert_eq!(final_tier(Some(Tier::Key), true), Tier::Key);
    assert_eq!(final_tier(Some(Tier::Skim), false), Tier::Skim);
}

#[test]
fn a_file_listed_twice_is_refused() {
    let list = [
        mark("api", "a.rs", Tier::Key),
        mark("api", "b.rs", Tier::Skim),
        mark("api", "a.rs", Tier::Skim),
    ];
    assert_eq!(
        check_importance(&list),
        Err(ImportanceError::Duplicate {
            repo: "api".to_owned(),
            path: "a.rs".to_owned()
        })
    );
    assert_eq!(check_importance(&list[..2]), Ok(()));
    // The same path in two repos is two files.
    assert_eq!(
        check_importance(&[mark("api", "a.rs", Tier::Key), mark("web", "a.rs", Tier::Key)]),
        Ok(())
    );
}

// -- arrangement -----------------------------------------------------------

#[test]
fn with_no_order_key_files_come_first_in_each_repo_and_skims_fold_last() {
    let diffs = [
        fake_diff("api", &["a.rs", "b.rs", "c.rs", "d.rs", "e.rs"]),
        fake_diff("web", &["x.js", "y.js"]),
    ];
    let plan = ReviewPlan::default();
    let tiers = [
        ("api", "d.rs", Tier::Key),
        ("api", "b.rs", Tier::Key),
        ("api", "a.rs", Tier::Skim),
        ("web", "y.js", Tier::Key),
    ];
    assert_eq!(
        layout(&plan, &diffs, &tiers),
        [
            (None, false, strings(&["b.rs", "d.rs", "c.rs", "e.rs", "y.js", "x.js"])),
            (None, true, strings(&["a.rs"])),
        ]
    );
}

#[test]
fn skims_sit_after_their_own_repos_files() {
    let diffs = [
        fake_diff("api", &["a.rs", "lock"]),
        fake_diff("web", &["x.js", "y.lock"]),
    ];
    let sections = ReviewPlan::default().arrange_with(
        &diffs,
        &HashMap::from([(file("api", "lock"), Tier::Skim), (file("web", "y.lock"), Tier::Skim)]),
    );
    let repos: Vec<(&str, bool)> = sections
        .sections
        .iter()
        .map(|section| (section.files[0].repo.as_str(), section.skim))
        .collect();
    // One plain section holds both repos' files, so each skim follows the
    // last section its repo has, in repo order.
    assert_eq!(repos, [("api", false), ("api", true), ("web", true)]);
}

#[test]
fn with_an_order_key_files_stay_where_it_put_them() {
    let diffs = [fake_diff("api", &["a.rs", "b.rs", "c.rs", "d.rs"])];
    let plan = ReviewPlan {
        order: vec![file("api", "c.rs"), file("api", "a.rs"), file("api", "b.rs")],
        ..ReviewPlan::default()
    };
    assert_eq!(
        layout(&plan, &diffs, &[("api", "b.rs", Tier::Key), ("api", "d.rs", Tier::Key)]),
        [(None, false, strings(&["c.rs", "a.rs", "b.rs", "d.rs"]))]
    );
}

#[test]
fn skim_files_leave_their_groups_and_an_emptied_group_draws_no_header() {
    let diffs = [fake_diff("api", &["a.rs", "b.rs", "lock", "c.rs"])];
    let group = |title: &str, first: &str| Group {
        title: title.to_owned(),
        note: String::new(),
        first: file("api", first),
    };
    let plan = ReviewPlan {
        order: vec![
            file("api", "a.rs"),
            file("api", "b.rs"),
            file("api", "lock"),
            file("api", "c.rs"),
        ],
        groups: vec![group("Core", "a.rs"), group("Generated", "lock"), group("Tail", "c.rs")],
        ..ReviewPlan::default()
    };
    let arranged = plan.arrange_with(&diffs, &HashMap::from([(file("api", "lock"), Tier::Skim)]));
    let shape: Vec<(Option<&str>, bool, Vec<&str>)> = arranged
        .sections
        .iter()
        .map(|section| (section.title.as_deref(), section.skim, names(&section.files)))
        .collect();
    assert_eq!(
        shape,
        [
            (Some("Core"), false, vec!["a.rs", "b.rs"]),
            (Some("Tail"), false, vec!["c.rs"]),
            (None, true, vec!["lock"]),
        ]
    );
}

#[test]
fn arrange_without_tiers_uses_the_plans_own_marks() {
    let diffs = [fake_diff("api", &["a.rs", "b.rs", "c.rs"])];
    let plan = ReviewPlan {
        importance: vec![mark("api", "c.rs", Tier::Key), mark("api", "a.rs", Tier::Skim)],
        ..ReviewPlan::default()
    };
    let sections = plan.arrange(&diffs).sections;
    assert_eq!(names(&sections[0].files), ["c.rs", "b.rs"]);
    assert!(sections[1].skim);
}

// -- picked layer and whole stack -------------------------------------------

fn layer(title: &str, range: (&str, &str), order: &[&str], importance: Vec<Importance>) -> Layer {
    Layer {
        title: title.to_owned(),
        repos: vec![LayerRepo {
            repo: "api".to_owned(),
            part: Part::Range {
                base: range.0.to_owned(),
                head: range.1.to_owned(),
            },
            pull_request: None,
        }],
        order: order.iter().map(|path| file("api", path)).collect(),
        importance,
        ..Layer::default()
    }
}

fn checked(layers: Vec<Layer>) -> humata2_review_diff::CheckedStack {
    let repo = Repo::new();
    repo.write("a.txt", "base\n");
    repo.commit_all("base");
    repo.git(&["checkout", "-q", "-b", "work"]);
    repo.write("b.txt", "work\n");
    repo.commit_all("work");
    repo.git(&["checkout", "-q", "-b", "work2"]);
    repo.write("c.txt", "more\n");
    repo.commit_all("work2");
    let clones = Clones::new(BTreeMap::from([("api".to_owned(), repo.checkout())]));
    let stack = Stack::new(layers).check_blocking(&clones).expect("a valid stack");
    // The repo folder lives as long as the check needs; resolved commits stay.
    drop(repo);
    stack
}

#[test]
fn a_picked_layer_reads_by_its_own_order_groups_and_importance() {
    let mut one = layer("one", ("main", "work"), &["b", "a"], vec![mark("api", "a", Tier::Skim)]);
    one.groups = vec![Group {
        title: "First".to_owned(),
        note: String::new(),
        first: file("api", "b"),
    }];
    let two = layer("two", ("work", "work2"), &["c"], vec![]);
    let stack = checked(vec![one.clone(), two]);
    let picked = stack.layer_plan("ONE").expect("layer one");
    assert_eq!(picked.order, one.order);
    assert_eq!(picked.groups, one.groups);
    assert_eq!(picked.importance, one.importance);
    // The plan's own order plays no part: a picked layer has no way to see it.
    let other = stack.layer_plan("two").expect("layer two");
    assert_eq!(names(&other.order), ["c"]);
    assert!(other.groups.is_empty());
    assert!(stack.layer_plan("missing").is_err());
}

#[test]
fn the_whole_stack_joins_layer_orders_bottom_first_and_merges_importance() {
    let bottom = layer(
        "bottom",
        ("main", "work"),
        &["a", "b", "lock"],
        vec![
            mark("api", "b", Tier::Skim),
            mark("api", "lock", Tier::Skim),
            mark("api", "c", Tier::Skim),
        ],
    );
    let top = layer(
        "top",
        ("work", "work2"),
        &["c", "a", "d"],
        vec![
            mark("api", "a", Tier::Key),
            mark("api", "lock", Tier::Skim),
            mark("api", "d", Tier::Skim),
        ],
    );
    let plan = checked(vec![bottom, top]).whole_plan();
    assert_eq!(names(&plan.order), ["a", "b", "lock", "c", "d"]);
    assert!(plan.groups.is_empty());
    let got: Vec<(&str, Tier)> = plan
        .importance
        .iter()
        .map(|entry| (entry.file.path.as_str(), entry.tier))
        .collect();
    // a: key in one layer wins. b: only the bottom touches it and skims.
    // lock: both skim. c: skim below, normal (in the order only) above.
    // d: only the top touches it and skims.
    assert_eq!(
        got,
        [("a", Tier::Key), ("b", Tier::Skim), ("lock", Tier::Skim), ("d", Tier::Skim)]
    );
}
