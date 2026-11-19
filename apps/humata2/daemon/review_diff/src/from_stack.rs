//! A stack read from a repo's own stack metadata (D803): the daemon runs
//! `stack status --json --offline` in the session's checkout and turns
//! each topic into a [`crate::Layer`], so the agent names only the repo.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use serde_json::Value;
use thiserror::Error;

use crate::stack::{Drift, DriftKind, Layer, LayerRepo, Part};
use crate::{Checkout, DiffError};

/// What `review_prepare {from_stack}` carries, kept as sent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FromStack {
    /// The repo whose stack to read, by its session name.
    pub repo: String,
    /// The topic the layers end at; needed when the stack forks.
    pub tip: Option<String>,
    /// Per topic name: the agent's title and purpose.
    pub topics: BTreeMap<String, TopicOverride>,
}

/// The agent's words for one topic's layer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TopicOverride {
    pub title: Option<String>,
    pub purpose: Option<String>,
}

const MAX_TOPICS: usize = 50;

/// Runs `stack status --json` in a directory. The seam lets tests (and the
/// daemon's own tests) stand in for the real tool.
pub trait StackStatus: Send + Sync {
    /// The tool's stdout. `online` false runs it with `--offline`.
    ///
    /// # Errors
    ///
    /// [`StatusError`] when the tool can't run or exits non-zero.
    fn status(&self, dir: &Path, online: bool) -> Result<String, StatusError>;
}

/// Why the stack tool's status couldn't be read.
#[derive(Debug, Error)]
pub enum StatusError {
    #[error("the `stack` tool is not installed on this machine")]
    NotInstalled,

    #[error("`stack status` exited with {status:?}: {stderr}")]
    Failed { status: Option<i32>, stderr: String },

    #[error("could not run the `stack` tool")]
    Spawn {
        #[source]
        source: std::io::Error,
    },
}

/// The real tool: the `stack` binary on PATH.
#[derive(Clone, Copy, Debug, Default)]
pub struct StackBinary;

impl StackStatus for StackBinary {
    fn status(&self, dir: &Path, online: bool) -> Result<String, StatusError> {
        let mut command = Command::new("stack");
        command.current_dir(dir).args(["status", "--json"]);
        if !online {
            command.arg("--offline");
        }
        let output = command.output().map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                StatusError::NotInstalled
            } else {
                StatusError::Spawn { source }
            }
        })?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(StatusError::Failed {
                status: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            })
        }
    }
}

/// Why a stack couldn't be read from the repo's metadata.
#[derive(Debug, Error)]
pub enum FromStackError {
    #[error(
        "the stack forks; its leaf topics are {}; send `tip` as one of them to review that chain",
        leaves.join(", ")
    )]
    Fork { leaves: Vec<String> },

    #[error(
        "`tip` names `{tip}`, which is not a topic of the stack (topics: {}); send one of those",
        known.join(", ")
    )]
    UnknownTip { tip: String, known: Vec<String> },

    #[error(
        "`{topic}` is not a topic of the stack (topics: {}); use one of those names or drop the entry",
        known.join(", ")
    )]
    UnknownTopic { topic: String, known: Vec<String> },

    #[error("a restack is in progress; finish or abort it, then send again")]
    Restack,

    #[error("the stack has no topics; add Topic trailers to the commits, or send explicit layers")]
    NoTopics,

    #[error(
        "the stack has {count} topics and at most {max} are allowed; review a shorter chain with `tip`, or send fewer layers"
    )]
    TooManyTopics { count: usize, max: usize },

    #[error("the `stack` tool is not installed on this machine; install it, then send again")]
    NotInstalled,

    #[error(
        "the `stack` tool here is too old to report each topic's head; upgrade it (Humata offers a Migrate item in the Inbox), then send again"
    )]
    Outdated,

    #[error("`stack status` failed ({status:?}): {stderr}; fix what it reports, then send again")]
    StatusFailed { status: Option<i32>, stderr: String },

    #[error("could not run the `stack` tool; check it is executable, then send again")]
    Spawn {
        #[source]
        source: std::io::Error,
    },

    #[error("topic `{topic}` has no commits to show yet; build the stack so it has a head, then send again")]
    NoHead { topic: String },

    #[error("`stack status` printed something unreadable; upgrade the `stack` tool, then send again")]
    BadJson {
        #[source]
        source: serde_json::Error,
    },

    #[error("`stack status` printed an unexpected shape ({reason}); upgrade the `stack` tool, then send again")]
    Malformed { reason: String },

    #[error("git failed while reading the stack")]
    Git {
        #[source]
        source: DiffError,
    },

    #[error(
        "layer `{layer}` is a range in `{repo}`, which the stack already covers; make it a topic, or send that repo as explicit layers"
    )]
    RangeInStackRepo { layer: String, repo: String },
}

impl From<StatusError> for FromStackError {
    fn from(error: StatusError) -> Self {
        match error {
            StatusError::NotInstalled => Self::NotInstalled,
            StatusError::Failed { status, stderr } => Self::StatusFailed { status, stderr },
            StatusError::Spawn { source } => Self::Spawn { source },
        }
    }
}

struct Topic {
    name: String,
    parent: Option<String>,
    head: String,
    base: String,
}

impl FromStack {
    /// The layers of the stack read in `checkout`, root to tip, each head
    /// pinned under `refs/humata/stack/<topic>`.
    ///
    /// # Errors
    ///
    /// [`FromStackError`] naming what to fix.
    pub fn read_blocking(
        &self,
        checkout: &Checkout,
        runner: &dyn StackStatus,
    ) -> Result<Vec<Layer>, FromStackError> {
        let text = runner.status(checkout.path(), false)?;
        let topics = self.chain(&text)?;
        let mut layers = Vec::with_capacity(topics.len());
        for topic in topics {
            let range = format!("{}..{}", topic.base, topic.head);
            let log = checkout
                .git
                .run(["log", "--reverse", "--format=%s", &range])
                .map_err(|source| FromStackError::Git { source })?;
            checkout
                .git
                .run([
                    "update-ref",
                    &format!("refs/humata/stack/{}", topic.name),
                    &topic.head,
                ])
                .map_err(|source| FromStackError::Git { source })?;
            let subject = String::from_utf8_lossy(&log)
                .lines()
                .next()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                // A layer title is at most 60 characters and unique
                // (D800): long subjects are cut, a repeated one falls back
                // to the topic name, which is unique.
                .map(|line| line.chars().take(60).collect::<String>())
                .filter(|line| {
                    !layers
                        .iter()
                        .any(|layer: &Layer| layer.title.eq_ignore_ascii_case(line))
                });
            let over = self.topics.get(&topic.name);
            layers.push(Layer {
                title: over
                    .and_then(|over| over.title.clone())
                    .or(subject)
                    .unwrap_or_else(|| topic.name.clone()),
                purpose: over.and_then(|over| over.purpose.clone()),
                repos: vec![LayerRepo {
                    repo: self.repo.clone(),
                    part: Part::Range {
                        base: topic.base,
                        head: topic.head,
                    },
                    pull_request: None,
                }],
                topic: Some(topic.name),
                ..Layer::default()
            });
        }
        Ok(layers)
    }

    /// Parses the status JSON and picks the chain, root first.
    fn chain(&self, text: &str) -> Result<Vec<Topic>, FromStackError> {
        let json: Value =
            serde_json::from_str(text).map_err(|source| FromStackError::BadJson { source })?;
        if json.get("restack").is_some_and(|restack| !restack.is_null()) {
            return Err(FromStackError::Restack);
        }
        let items = json
            .get("topics")
            .and_then(Value::as_array)
            .ok_or_else(|| FromStackError::Malformed {
                reason: "no `topics` list".to_owned(),
            })?;
        if items.is_empty() {
            return Err(FromStackError::NoTopics);
        }
        if items.len() > MAX_TOPICS {
            return Err(FromStackError::TooManyTopics {
                count: items.len(),
                max: MAX_TOPICS,
            });
        }
        let mut topics = Vec::with_capacity(items.len());
        for item in items {
            let name = item
                .get("topic")
                .and_then(Value::as_str)
                .ok_or_else(|| FromStackError::Malformed {
                    reason: "a topic has no name".to_owned(),
                })?
                .to_owned();
            if item.get("head").is_none() || item.get("base_commit").is_none() {
                return Err(FromStackError::Outdated);
            }
            let text = |key: &str| item.get(key).and_then(Value::as_str).map(str::to_owned);
            let (Some(head), Some(base)) = (text("head"), text("base_commit")) else {
                return Err(FromStackError::NoHead { topic: name });
            };
            topics.push(Topic {
                parent: text("parent"),
                name,
                head,
                base,
            });
        }
        let known: Vec<String> = topics.iter().map(|topic| topic.name.clone()).collect();
        if let Some(topic) = self.topics.keys().find(|key| !known.contains(key)) {
            return Err(FromStackError::UnknownTopic {
                topic: topic.clone(),
                known,
            });
        }
        let tip = match &self.tip {
            Some(tip) => {
                if !known.contains(tip) {
                    return Err(FromStackError::UnknownTip {
                        tip: tip.clone(),
                        known,
                    });
                }
                tip.clone()
            }
            None => {
                let leaves: Vec<String> = topics
                    .iter()
                    .filter(|topic| {
                        !topics
                            .iter()
                            .any(|other| other.parent.as_deref() == Some(&topic.name))
                    })
                    .map(|topic| topic.name.clone())
                    .collect();
                match <[String; 1]>::try_from(leaves) {
                    Ok([leaf]) => leaf,
                    Err(leaves) if leaves.is_empty() => {
                        return Err(FromStackError::Malformed {
                            reason: "the topics' parents form a loop".to_owned(),
                        });
                    }
                    Err(leaves) => return Err(FromStackError::Fork { leaves }),
                }
            }
        };
        let mut by_name: BTreeMap<String, Topic> = topics
            .into_iter()
            .map(|topic| (topic.name.clone(), topic))
            .collect();
        let mut chain = Vec::new();
        let mut next = Some(tip);
        while let Some(name) = next {
            let Some(topic) = by_name.remove(&name) else {
                return Err(FromStackError::Malformed {
                    reason: "the topics' parents form a loop".to_owned(),
                });
            };
            next = topic.parent.clone().filter(|parent| by_name.contains_key(parent));
            chain.push(topic);
        }
        chain.reverse();
        Ok(chain)
    }

    /// The stack's layers first, then the explicit ones.
    ///
    /// # Errors
    ///
    /// [`FromStackError::RangeInStackRepo`] when an explicit layer holds a
    /// range in `repo`, the repo the stack was read from.
    pub fn merge(
        from: Vec<Layer>,
        explicit: Vec<Layer>,
        repo: &str,
    ) -> Result<Vec<Layer>, FromStackError> {
        for layer in &explicit {
            if layer
                .repos
                .iter()
                .any(|part| part.repo == repo && matches!(part.part, Part::Range { .. }))
            {
                return Err(FromStackError::RangeInStackRepo {
                    layer: layer.title.clone(),
                    repo: repo.to_owned(),
                });
            }
        }
        let mut layers = from;
        layers.extend(explicit);
        Ok(layers)
    }

    /// What moved between two readings, matched by topic name. Layers
    /// without a topic are ignored.
    #[must_use]
    pub fn drift(kept: &[Layer], now: &[Layer]) -> Vec<Drift> {
        let mut drift = Vec::new();
        let mut top: Option<&Layer> = None;
        for layer in kept {
            let Some(topic) = &layer.topic else { continue };
            top = Some(layer);
            let Some(repo) = layer.repos.first() else { continue };
            let current = now.iter().find(|other| other.topic.as_ref() == Some(topic));
            let kind = match current {
                None => Some(DriftKind::Missing),
                Some(current) if current.repos != layer.repos => Some(DriftKind::Moved),
                Some(_) => None,
            };
            if let Some(kind) = kind {
                drift.push(Drift {
                    layer: layer.title.clone(),
                    repo: repo.repo.clone(),
                    kind,
                });
            }
        }
        let grew = now.iter().any(|layer| {
            layer.topic.is_some()
                && !kept.iter().any(|other| other.topic == layer.topic)
        });
        if let (true, Some(top)) = (grew, top) {
            if let Some(repo) = top.repos.first() {
                drift.push(Drift {
                    layer: top.title.clone(),
                    repo: repo.repo.clone(),
                    kind: DriftKind::HeadMoved,
                });
            }
        }
        drift
    }

    /// `now` with the kept layers' titles and purposes (the agent's words)
    /// carried over by topic name: new ranges adopted, new topics appended,
    /// missing ones dropped.
    #[must_use]
    pub fn refresh(kept: &[Layer], now: Vec<Layer>) -> Vec<Layer> {
        now.into_iter()
            .map(|mut layer| {
                let old = kept
                    .iter()
                    .find(|old| old.topic.is_some() && old.topic == layer.topic);
                if let Some(old) = old {
                    layer.title.clone_from(&old.title);
                    layer.purpose.clone_from(&old.purpose);
                }
                layer
            })
            .collect()
    }
}
