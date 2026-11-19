//! D802: the tiers a plan gives files stay with the review chat. The app's
//! reports never carry them, so a report keeps the plan's; Undo puts back
//! the tiers before a plan; and the tiers served for the view follow the
//! layer showing.

use humata2_review_diff::{
    DiffSpec, FileRef, Importance, Layer, LayerRepo, Part, ReviewChat, ReviewPlan, Stack, Tier,
    ViewState,
};

fn file(path: &str) -> FileRef {
    FileRef {
        repo: "acme/api".to_owned(),
        path: path.to_owned(),
    }
}

fn tier(path: &str, tier: Tier) -> Importance {
    Importance { file: file(path), tier }
}

fn plan(importance: Vec<Importance>) -> ReviewPlan {
    ReviewPlan {
        spec: DiffSpec::default(),
        importance,
        ..ReviewPlan::default()
    }
}

fn layer(title: &str, order: &[&str], importance: Vec<Importance>) -> Layer {
    Layer {
        title: title.to_owned(),
        purpose: None,
        repos: vec![LayerRepo {
            repo: "acme/api".to_owned(),
            part: Part::Range {
                base: "main".to_owned(),
                head: title.to_owned(),
            },
            pull_request: None,
        }],
        order: order.iter().map(|path| file(path)).collect(),
        importance,
        ..Layer::default()
    }
}

fn open() -> ReviewChat {
    let mut chat = ReviewChat::default();
    chat.report(ViewState::default());
    chat
}

#[test]
fn a_report_from_the_app_keeps_the_plans_tiers() {
    let mut chat = open();
    chat.prepare("Claude Code", plan(vec![tier("src/codec.rs", Tier::Key)]))
        .expect("the plan applies");
    chat.report(ViewState::default());
    assert_eq!(chat.view().agent_importance(), vec![tier("src/codec.rs", Tier::Key)]);
}

#[test]
fn undoing_a_plan_puts_back_the_tiers_before_it() {
    let mut chat = open();
    chat.prepare("Claude Code", plan(vec![tier("Cargo.lock", Tier::Normal)]))
        .expect("the first plan");
    let change = chat
        .prepare("Claude Code", plan(vec![tier("src/codec.rs", Tier::Key)]))
        .expect("the second plan");
    chat.undo(&change.undo_id).expect("undo the second");
    assert_eq!(chat.view().agent_importance(), vec![tier("Cargo.lock", Tier::Normal)]);
}

#[test]
fn a_picked_layer_serves_its_own_tiers_and_whole_stack_the_joined_ones() {
    let mut chat = open();
    let stack = Stack::new(vec![
        layer("codec", &["src/codec.rs"], vec![tier("Cargo.lock", Tier::Skim)]),
        layer("journal", &["src/journal.rs"], vec![tier("src/journal.rs", Tier::Key)]),
    ]);
    chat.prepare(
        "Claude Code",
        ReviewPlan {
            stack: Some(stack),
            ..plan(Vec::new())
        },
    )
    .expect("the stack applies");
    let whole = chat.view().agent_importance();
    assert!(whole.contains(&tier("src/journal.rs", Tier::Key)));
    assert!(whole.contains(&tier("Cargo.lock", Tier::Skim)));

    chat.focus_layer(Some("codec".to_owned())).expect("to codec");
    assert_eq!(chat.view().agent_importance(), vec![tier("Cargo.lock", Tier::Skim)]);
}
