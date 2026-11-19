//! A stack read from the repo's own stack metadata (D803): layers in chain
//! order from a status JSON and a fake runner, over a real temp repo for
//! commit subjects and pinned refs.

mod support;

use std::collections::BTreeMap;
use std::path::Path;

use humata2_review_diff::{
    DriftKind, FromStack, FromStackError, Layer, LayerRepo, Part, StackStatus, StatusError,
    TopicOverride,
};
use support::Repo;

enum Fake {
    Out(String),
    Missing,
    Fails,
}

impl StackStatus for Fake {
    fn status(&self, _dir: &Path, online: bool) -> Result<String, StatusError> {
        assert!(!online, "reading a stack never goes online");
        match self {
            Self::Out(text) => Ok(text.clone()),
            Self::Missing => Err(StatusError::NotInstalled),
            Self::Fails => Err(StatusError::Failed {
                status: Some(2),
                stderr: "boom".to_owned(),
            }),
        }
    }
}

/// trunk, then `codec`, `journal`, `ui` one commit each, linearly.
struct Chain {
    repo: Repo,
    /// (topic, base, head)
    topics: Vec<(String, String, String)>,
}

fn chain() -> Chain {
    let repo = Repo::new();
    repo.write("a.txt", "trunk\n");
    let mut base = repo.commit_all("trunk");
    let mut topics = Vec::new();
    for (topic, subject) in [
        ("codec", "Add the codec"),
        ("journal", "Add the journal log"),
        ("ui", "Wire the UI"),
    ] {
        repo.write(&format!("{topic}.txt"), "x\n");
        let head = repo.commit_all(subject);
        topics.push((topic.to_owned(), base.clone(), head.clone()));
        base = head;
    }
    Chain { repo, topics }
}

fn entry(topic: &str, parent: Option<&str>, base: &str, head: &str) -> String {
    let parent = parent.map_or("null".to_owned(), |p| format!("\"{p}\""));
    format!(
        "{{\"topic\":\"{topic}\",\"parent\":{parent},\"depth\":0,\"branch\":\"b\",\"base\":\"main\",\"head\":\"{head}\",\"base_commit\":\"{base}\",\"number\":null,\"url\":null,\"state\":null,\"needs_upload\":false}}"
    )
}

fn status(entries: &[String], restack: &str) -> String {
    format!(
        "{{\"trunk\":\"main\",\"topics\":[{}],\"orphaned\":[],\"restack\":{restack}}}",
        entries.join(",")
    )
}

fn linear(chain: &Chain) -> String {
    let entries: Vec<String> = chain
        .topics
        .iter()
        .enumerate()
        .map(|(i, (topic, base, head))| {
            let parent = (i > 0).then(|| chain.topics[i - 1].0.as_str());
            entry(topic, parent, base, head)
        })
        .collect();
    status(&entries, "null")
}

fn from(tip: Option<&str>) -> FromStack {
    FromStack {
        repo: "api".to_owned(),
        tip: tip.map(str::to_owned),
        topics: BTreeMap::new(),
    }
}

fn read(
    chain: &Chain,
    from: &FromStack,
    fake: &Fake,
) -> Result<Vec<Layer>, FromStackError> {
    from.read_blocking(&chain.repo.checkout(), fake)
}

fn topics_of(layers: &[Layer]) -> Vec<Option<&str>> {
    layers.iter().map(|layer| layer.topic.as_deref()).collect()
}

#[test]
fn a_linear_stack_gives_layers_in_chain_order_titled_by_first_commit() {
    let chain = chain();
    let layers = read(&chain, &from(None), &Fake::Out(linear(&chain))).expect("reads");
    assert_eq!(
        topics_of(&layers),
        [Some("codec"), Some("journal"), Some("ui")]
    );
    let titles: Vec<&str> = layers.iter().map(|l| l.title.as_str()).collect();
    assert_eq!(titles, ["Add the codec", "Add the journal log", "Wire the UI"]);
    let (_, base, head) = &chain.topics[1];
    assert_eq!(
        layers[1].repos,
        [LayerRepo {
            repo: "api".to_owned(),
            part: Part::Range {
                base: base.clone(),
                head: head.clone()
            },
            pull_request: None,
        }]
    );
    assert_eq!(layers[1].purpose, None);
}

#[test]
fn each_head_is_pinned_under_its_own_ref() {
    let chain = chain();
    read(&chain, &from(None), &Fake::Out(linear(&chain))).expect("reads");
    for (topic, _, head) in &chain.topics {
        let pinned = chain
            .repo
            .git(&["rev-parse", &format!("refs/humata/stack/{topic}")]);
        assert_eq!(&pinned, head);
    }
}

#[test]
fn overrides_win_over_the_commit_subject() {
    let chain = chain();
    let mut request = from(None);
    request.topics.insert(
        "journal".to_owned(),
        TopicOverride {
            title: Some("The journal".to_owned()),
            purpose: Some("Keeps a log".to_owned()),
        },
    );
    let layers = read(&chain, &request, &Fake::Out(linear(&chain))).expect("reads");
    assert_eq!(layers[1].title, "The journal");
    assert_eq!(layers[1].purpose.as_deref(), Some("Keeps a log"));
    assert_eq!(layers[0].title, "Add the codec");
}

#[test]
fn an_unknown_override_is_refused_listing_the_topics() {
    let chain = chain();
    let mut request = from(None);
    request
        .topics
        .insert("nope".to_owned(), TopicOverride::default());
    let error = read(&chain, &request, &Fake::Out(linear(&chain))).expect_err("refused");
    let FromStackError::UnknownTopic { topic, known } = error else {
        panic!("wrong error: {error:?}");
    };
    assert_eq!(topic, "nope");
    assert_eq!(known, ["codec", "journal", "ui"]);
}

fn forked(chain: &Chain) -> String {
    let (_, b0, h0) = &chain.topics[0];
    let (_, _, h1) = &chain.topics[1];
    let (_, _, h2) = &chain.topics[2];
    status(
        &[
            entry("codec", None, b0, h0),
            entry("journal", Some("codec"), h0, h1),
            entry("ui", Some("codec"), h0, h2),
        ],
        "null",
    )
}

#[test]
fn a_fork_is_refused_without_a_tip_and_read_to_the_tip_with_one() {
    let chain = chain();
    let error = read(&chain, &from(None), &Fake::Out(forked(&chain))).expect_err("refused");
    let FromStackError::Fork { leaves } = error else {
        panic!("wrong error: {error:?}");
    };
    assert_eq!(leaves, ["journal", "ui"]);

    let layers = read(&chain, &from(Some("ui")), &Fake::Out(forked(&chain))).expect("reads");
    assert_eq!(topics_of(&layers), [Some("codec"), Some("ui")]);
}

#[test]
fn a_tip_that_is_not_a_topic_is_refused() {
    let chain = chain();
    let error = read(&chain, &from(Some("zzz")), &Fake::Out(linear(&chain))).expect_err("refused");
    assert!(matches!(error, FromStackError::UnknownTip { ref tip, .. } if tip == "zzz"));
}

#[test]
fn a_restack_in_progress_is_refused() {
    let chain = chain();
    let text = linear(&chain).replace("\"restack\":null", "\"restack\":{\"topic\":\"codec\"}");
    let error = read(&chain, &from(None), &Fake::Out(text)).expect_err("refused");
    assert!(matches!(error, FromStackError::Restack));
}

#[test]
fn no_topics_is_refused() {
    let chain = chain();
    let error = read(&chain, &from(None), &Fake::Out(status(&[], "null"))).expect_err("refused");
    assert!(matches!(error, FromStackError::NoTopics));
}

#[test]
fn more_than_fifty_topics_is_refused() {
    let chain = chain();
    let (_, base, head) = &chain.topics[0];
    let entries: Vec<String> = (0..51)
        .map(|i| entry(&format!("t{i}"), None, base, head))
        .collect();
    let error =
        read(&chain, &from(None), &Fake::Out(status(&entries, "null"))).expect_err("refused");
    assert!(matches!(
        error,
        FromStackError::TooManyTopics { count: 51, max: 50 }
    ));
}

#[test]
fn json_without_head_is_outdated() {
    let chain = chain();
    let old = "{\"trunk\":\"main\",\"topics\":[{\"topic\":\"codec\",\"parent\":null,\"depth\":0,\"branch\":\"b\",\"base\":\"main\",\"number\":null,\"url\":null,\"state\":null,\"needs_upload\":false}],\"orphaned\":[],\"restack\":null}";
    let error = read(&chain, &from(None), &Fake::Out(old.to_owned())).expect_err("refused");
    assert!(matches!(error, FromStackError::Outdated));
}

#[test]
fn a_topic_with_a_null_head_is_refused_by_name() {
    let chain = chain();
    let text = linear(&chain).replacen(
        &format!("\"head\":\"{}\"", chain.topics[0].2),
        "\"head\":null",
        1,
    );
    let error = read(&chain, &from(None), &Fake::Out(text)).expect_err("refused");
    assert!(matches!(error, FromStackError::NoHead { ref topic } if topic == "codec"));
}

#[test]
fn unreadable_json_is_refused() {
    let chain = chain();
    let error = read(&chain, &from(None), &Fake::Out("not json".to_owned())).expect_err("refused");
    assert!(matches!(error, FromStackError::BadJson { .. }));
}

#[test]
fn the_runner_failing_maps_to_its_own_variants() {
    let chain = chain();
    let error = read(&chain, &from(None), &Fake::Missing).expect_err("refused");
    assert!(matches!(error, FromStackError::NotInstalled));
    let error = read(&chain, &from(None), &Fake::Fails).expect_err("refused");
    assert!(matches!(
        error,
        FromStackError::StatusFailed { status: Some(2), .. }
    ));
}

fn range_layer(repo: &str) -> Layer {
    Layer {
        title: "explicit".to_owned(),
        repos: vec![LayerRepo {
            repo: repo.to_owned(),
            part: Part::Range {
                base: "a".to_owned(),
                head: "b".to_owned(),
            },
            pull_request: None,
        }],
        ..Layer::default()
    }
}

#[test]
fn merge_puts_stack_layers_first_and_refuses_a_range_in_the_same_repo() {
    let chain = chain();
    let layers = read(&chain, &from(None), &Fake::Out(linear(&chain))).expect("reads");

    let merged =
        FromStack::merge(layers.clone(), vec![range_layer("web")], "api").expect("other repo");
    assert_eq!(merged.len(), 4);
    assert_eq!(merged[3].title, "explicit");

    let error = FromStack::merge(layers.clone(), vec![range_layer("api")], "api")
        .expect_err("refused");
    assert!(matches!(error, FromStackError::RangeInStackRepo { .. }));

    let worktree = Layer {
        title: "wip".to_owned(),
        repos: vec![LayerRepo {
            repo: "api".to_owned(),
            part: Part::Worktree {
                include_paths: vec![".".to_owned()],
            },
            pull_request: None,
        }],
        ..Layer::default()
    };
    assert!(FromStack::merge(layers, vec![worktree], "api").is_ok());
}

fn topic_layer(topic: &str, base: &str, head: &str) -> Layer {
    Layer {
        title: format!("title {topic}"),
        topic: Some(topic.to_owned()),
        repos: vec![LayerRepo {
            repo: "api".to_owned(),
            part: Part::Range {
                base: base.to_owned(),
                head: head.to_owned(),
            },
            pull_request: None,
        }],
        ..Layer::default()
    }
}

#[test]
fn drift_reports_moved_missing_and_head_moved_by_topic() {
    let kept = vec![
        topic_layer("a", "0", "1"),
        topic_layer("b", "1", "2"),
        topic_layer("c", "2", "3"),
    ];
    let now = vec![
        topic_layer("a", "0", "1"),
        topic_layer("b", "1", "22"),
        topic_layer("d", "3", "4"),
    ];
    let drift = FromStack::drift(&kept, &now);
    let found: Vec<(&str, DriftKind)> = drift
        .iter()
        .map(|d| (d.layer.as_str(), d.kind))
        .collect();
    assert_eq!(
        found,
        [
            ("title b", DriftKind::Moved),
            ("title c", DriftKind::Missing),
            ("title c", DriftKind::HeadMoved),
        ]
    );
    assert!(FromStack::drift(&kept, &kept).is_empty());
}

#[test]
fn refresh_adopts_new_ranges_keeps_the_agents_words_and_drops_missing() {
    let mut kept_b = topic_layer("b", "1", "2");
    kept_b.title = "My title".to_owned();
    kept_b.purpose = Some("My purpose".to_owned());
    let kept = vec![topic_layer("a", "0", "1"), kept_b, topic_layer("c", "2", "3")];
    let now = vec![
        topic_layer("a", "0", "1"),
        topic_layer("b", "1", "22"),
        topic_layer("d", "22", "4"),
    ];
    let refreshed = FromStack::refresh(&kept, now);
    assert_eq!(
        topics_of(&refreshed),
        [Some("a"), Some("b"), Some("d")]
    );
    assert_eq!(refreshed[1].title, "My title");
    assert_eq!(refreshed[1].purpose.as_deref(), Some("My purpose"));
    assert_eq!(
        refreshed[1].repos[0].part,
        Part::Range {
            base: "1".to_owned(),
            head: "22".to_owned()
        }
    );
    assert_eq!(refreshed[2].title, "title d");
}
