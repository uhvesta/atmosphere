//! Restack behavior for stacked layers and `Looked at` preservation (DECISIONS.md §1):
//! if a layer's diff is unchanged after a restack, keep `Looked at` and quietly
//! note `Rebased · diff unchanged`; if its diff changed, invalidate `Looked at`.

mod support;

use std::collections::BTreeMap;

use humata2_review_diff::{
    Clones, Layer, LayerLook, LayerRepo, Look, Part, REBASED_DIFF_UNCHANGED_NOTE, RestackStatus,
    Stack, reconcile_looks_after_restack,
};
use support::Repo;

fn clones(repo: &Repo) -> Clones {
    Clones::new(BTreeMap::from([("app".to_owned(), repo.checkout())]))
}

fn range(base: &str, head: &str) -> LayerRepo {
    LayerRepo {
        repo: "app".to_owned(),
        part: Part::Range {
            base: base.to_owned(),
            head: head.to_owned(),
        },
        pull_request: None,
    }
}

fn layer(title: &str, base: &str, head: &str) -> Layer {
    Layer {
        title: title.to_owned(),
        purpose: None,
        repos: vec![range(base, head)],
        ..Layer::default()
    }
}

fn two_layer_stack() -> Stack {
    Stack::new(vec![
        layer("layer-1", "main", "layer-1"),
        layer("layer-2", "layer-1", "layer-2"),
    ])
}

/// Repo with `main` (base.txt), `layer-1` (+a.txt), and `layer-2` (+b.txt).
fn stacked_repo() -> Repo {
    let repo = Repo::new();
    repo.write("base.txt", "root\n");
    repo.commit_all("base");
    repo.git(&["checkout", "-q", "-b", "layer-1"]);
    repo.write("a.txt", "alpha\n");
    repo.commit_all("layer-1");
    repo.git(&["checkout", "-q", "-b", "layer-2"]);
    repo.write("b.txt", "beta\n");
    repo.commit_all("layer-2");
    repo
}

#[test]
fn rebased_layers_with_unchanged_diff_keep_looked_at_with_quiet_note() {
    let repo = stacked_repo();
    let stack = two_layer_stack();
    let before = stack
        .check_blocking(&clones(&repo))
        .expect("initial stack is valid");

    let checkout = repo.checkout();
    let look_1 = vec![Look {
        path: "a.txt".to_owned(),
        fingerprint: 101,
    }];
    let look_2 = vec![Look {
        path: "b.txt".to_owned(),
        fingerprint: 202,
    }];
    checkout
        .save_looks_blocking("s-layer-1", &look_1)
        .expect("save layer-1 looks");
    checkout
        .save_looks_blocking("s-layer-2", &look_2)
        .expect("save layer-2 looks");

    // Advance main with an unrelated commit c.txt, then rebase both layers.
    repo.git(&["checkout", "-q", "main"]);
    repo.write("c.txt", "unrelated\n");
    repo.commit_all("advance main");

    repo.git(&["checkout", "-q", "layer-1"]);
    repo.git(&["rebase", "-q", "main"]);
    repo.git(&["checkout", "-q", "layer-2"]);
    repo.git(&["rebase", "-q", "layer-1"]);

    let after = stack
        .check_blocking(&clones(&repo))
        .expect("rebased stack is valid");

    // Confirm commit SHAs actually moved.
    assert_ne!(
        before.resolved("layer-1", "app").expect("before l1").head_commit,
        after.resolved("layer-1", "app").expect("after l1").head_commit
    );
    assert_ne!(
        before.resolved("layer-2", "app").expect("before l2").head_commit,
        after.resolved("layer-2", "app").expect("after l2").head_commit
    );

    let statuses = before
        .restack_status_blocking(&clones(&repo), &after)
        .expect("compute restack status");
    assert_eq!(statuses.len(), 2);
    for entry in &statuses {
        assert_eq!(entry.status, RestackStatus::RebasedDiffUnchanged);
        assert_eq!(entry.status.note(), Some("Rebased · diff unchanged"));
    }

    let kept_2 = checkout
        .reconcile_looks_blocking("s-layer-2", statuses[1].status)
        .expect("reconcile layer-2 looks");
    assert_eq!(
        kept_2,
        vec![LayerLook {
            look: look_2[0].clone(),
            rebased_diff_unchanged: true,
        }]
    );
    assert_eq!(kept_2[0].note(), Some(REBASED_DIFF_UNCHANGED_NOTE));
    assert_eq!(
        checkout.looks_blocking("s-layer-2").expect("read back"),
        look_2
    );
}

#[test]
fn modified_layer_during_restack_invalidates_looked_at_while_unchanged_upper_layer_keeps_it() {
    let repo = stacked_repo();
    let stack = two_layer_stack();
    let before = stack
        .check_blocking(&clones(&repo))
        .expect("initial stack is valid");

    let checkout = repo.checkout();
    let look_1 = vec![Look {
        path: "a.txt".to_owned(),
        fingerprint: 101,
    }];
    let look_2 = vec![Look {
        path: "b.txt".to_owned(),
        fingerprint: 202,
    }];
    checkout
        .save_looks_blocking("s-layer-1", &look_1)
        .expect("save layer-1 looks");
    checkout
        .save_looks_blocking("s-layer-2", &look_2)
        .expect("save layer-2 looks");

    // Advance main, rebase layer-1 AND change its diff, then rebase layer-2 onto it.
    repo.git(&["checkout", "-q", "main"]);
    repo.write("c.txt", "unrelated\n");
    repo.commit_all("advance main");

    repo.git(&["checkout", "-q", "layer-1"]);
    repo.git(&["rebase", "-q", "main"]);
    repo.write("a.txt", "alpha-modified\n");
    repo.git(&["commit", "-q", "-a", "--amend", "-m", "layer-1 modified"]);

    repo.git(&["checkout", "-q", "layer-2"]);
    repo.git(&["rebase", "-q", "--onto", "layer-1", "HEAD~1", "layer-2"]);

    let after = stack
        .check_blocking(&clones(&repo))
        .expect("restacked stack is valid");

    let statuses = before
        .restack_status_blocking(&clones(&repo), &after)
        .expect("compute restack status");
    assert_eq!(statuses[0].layer, "layer-1");
    assert_eq!(statuses[0].status, RestackStatus::DiffChanged);
    assert_eq!(statuses[0].status.note(), None);

    assert_eq!(statuses[1].layer, "layer-2");
    assert_eq!(statuses[1].status, RestackStatus::RebasedDiffUnchanged);
    assert_eq!(statuses[1].status.note(), Some("Rebased · diff unchanged"));

    // layer-1 looks are dropped; layer-2 looks are preserved with quiet note.
    let reconciled_1 = checkout
        .reconcile_looks_blocking("s-layer-1", statuses[0].status)
        .expect("reconcile layer-1");
    assert!(reconciled_1.is_empty());
    assert!(
        checkout
            .looks_blocking("s-layer-1")
            .expect("read cleared looks")
            .is_empty()
    );

    let reconciled_2 = checkout
        .reconcile_looks_blocking("s-layer-2", statuses[1].status)
        .expect("reconcile layer-2");
    assert_eq!(reconciled_2.len(), 1);
    assert!(reconciled_2[0].rebased_diff_unchanged);
    assert_eq!(reconciled_2[0].note(), Some("Rebased · diff unchanged"));
    assert_eq!(
        checkout.looks_blocking("s-layer-2").expect("read kept looks"),
        look_2
    );
}

#[test]
fn unchanged_and_missing_layers_reconcile_cleanly() {
    let repo = stacked_repo();
    let stack = two_layer_stack();
    let checked = stack
        .check_blocking(&clones(&repo))
        .expect("initial stack is valid");

    let same = checked
        .restack_status_blocking(&clones(&repo), &checked)
        .expect("same stack");
    assert!(same.iter().all(|s| s.status == RestackStatus::Unchanged));

    let look = vec![Look {
        path: "a.txt".to_owned(),
        fingerprint: 1,
    }];
    let unchanged = reconcile_looks_after_restack(&look, RestackStatus::Unchanged);
    assert_eq!(
        unchanged,
        vec![LayerLook {
            look: look[0].clone(),
            rebased_diff_unchanged: false,
        }]
    );
    assert_eq!(unchanged[0].note(), None);

    let one_layer = Stack::new(vec![layer("layer-1", "main", "layer-1")])
        .check_blocking(&clones(&repo))
        .expect("one-layer stack");
    let dropped = checked
        .restack_status_blocking(&clones(&repo), &one_layer)
        .expect("missing layer-2");
    assert_eq!(dropped[0].status, RestackStatus::Unchanged);
    assert_eq!(dropped[1].status, RestackStatus::Missing);
    assert!(reconcile_looks_after_restack(&look, RestackStatus::Missing).is_empty());
}
