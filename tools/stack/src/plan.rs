//! Groups the branch's commits into topics and orders them so each topic
//! comes after the topic it is relative to.

use crate::trailers::Message;

/// A commit between the trunk and HEAD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Commit {
    /// The full sha.
    pub(crate) sha: String,
    /// The full sha of its only parent.
    pub(crate) parent: String,
    pub(crate) message: Message,
}

/// One pull request: every commit with the same Topic, in branch order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Topic {
    pub(crate) name: String,
    pub(crate) commits: Vec<Commit>,
    /// The topic this one builds on, from a `Relative` trailer.
    pub(crate) relative: Option<String>,
    /// From `Reviewers` trailers: split on commas and whitespace, without
    /// duplicates, in first-seen order, across all the topic's commits.
    pub(crate) reviewers: Vec<String>,
    /// From `Labels` trailers, the same way.
    pub(crate) labels: Vec<String>,
    /// True when any commit has `Draft: true` (or `yes`).
    pub(crate) draft: bool,
}

/// The topics in upload order and the commits without a Topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Plan {
    /// Every topic after its relative; otherwise in order of first commit.
    pub(crate) topics: Vec<Topic>,
    pub(crate) untagged: Vec<Commit>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PlanError {
    #[error(
        "commit {commit} has Topic `{name}`; topic names use only a-z, 0-9, `.`, `_` and `-`. \
         Reword the commit (git rebase -i, `reword`) to fix the trailer"
    )]
    BadTopicName { commit: String, name: String },

    #[error(
        "commit {commit} has more than one Topic ({names}); a commit belongs to one pull request. \
         Reword it to keep one Topic trailer, or split it (git rebase -i, `edit`)"
    )]
    ManyTopics { commit: String, names: String },

    #[error(
        "topic `{topic}` is relative to more than one topic ({relatives}); a pull request has one \
         base. Keep one Relative trailer across the topic's commits"
    )]
    ManyRelatives { topic: String, relatives: String },

    #[error(
        "topic `{topic}` is relative to `{relative}`, but no commit on this branch has Topic \
         `{relative}`. Add that topic's commits to this branch, or drop the Relative trailer to \
         base `{topic}` on the trunk"
    )]
    UnknownRelative { topic: String, relative: String },

    #[error("topic `{topic}` is relative to itself. Drop the Relative trailer, or name another topic")]
    SelfRelative { topic: String },

    #[error(
        "the topics {topics} are relative to each other in a loop, so none can go first. Drop one \
         of their Relative trailers"
    )]
    Cycle { topics: String },

    #[error("commit {commit} has `Draft: {value}`; use `Draft: true` or `Draft: false`")]
    BadDraft { commit: String, value: String },

    #[error("there is no topic `{name}` on this branch; `stack check` lists the topics")]
    UnknownTopic { name: String },
}

impl Plan {
    /// Groups commits (oldest first) into topics. Commits keep their branch
    /// order inside a topic, even when other commits sit between them.
    /// Commit shas in errors are the first 8 characters.
    pub(crate) fn build(commits: Vec<Commit>) -> Result<Plan, PlanError> {
        let mut untagged: Vec<Commit> = Vec::new();
        let mut topics: Vec<Topic> = Vec::new();
        for commit in commits {
            let name = commit_topic(&commit)?;
            let draft = commit_draft(&commit)?;
            let Some(name) = name else {
                untagged.push(commit);
                continue;
            };
            if let Some(topic) = topics.iter_mut().find(|topic| topic.name == name) {
                topic.draft |= draft;
                topic.commits.push(commit);
            } else {
                topics.push(Topic {
                    name,
                    commits: vec![commit],
                    relative: None,
                    reviewers: Vec::new(),
                    labels: Vec::new(),
                    draft,
                });
            }
        }

        let names: Vec<String> = topics.iter().map(|topic| topic.name.clone()).collect();
        for topic in &mut topics {
            let (relative, reviewers, labels) = topic_fields(topic, &names)?;
            topic.relative = relative;
            topic.reviewers = reviewers;
            topic.labels = labels;
        }
        Ok(Plan {
            topics: order(topics)?,
            untagged,
        })
    }

    /// Keeps only the named topics and every topic they are relative to,
    /// directly or not, in the same order.
    pub(crate) fn only(self, names: &[String]) -> Result<Plan, PlanError> {
        if names.is_empty() {
            return Ok(self);
        }
        if let Some(unknown) = names.iter().find(|name| self.topic(name.as_str()).is_none()) {
            return Err(PlanError::UnknownTopic {
                name: unknown.clone(),
            });
        }

        let mut keep: Vec<String> = Vec::new();
        let mut pending: Vec<String> = names.to_vec();
        while let Some(name) = pending.pop() {
            if keep.contains(&name) {
                continue;
            }
            if let Some(relative) = self.topic(&name).and_then(|topic| topic.relative.clone()) {
                pending.push(relative);
            }
            keep.push(name);
        }

        let topics: Vec<Topic> = self
            .topics
            .into_iter()
            .filter(|topic| keep.contains(&topic.name))
            .collect();
        Ok(Plan {
            topics,
            untagged: self.untagged,
        })
    }

    pub(crate) fn topic(&self, name: &str) -> Option<&Topic> {
        self.topics.iter().find(|topic| topic.name == name)
    }

    /// The topics directly relative to `name`, in plan order.
    pub(crate) fn children(&self, name: &str) -> Vec<&Topic> {
        self.topics
            .iter()
            .filter(|topic| topic.relative.as_deref() == Some(name))
            .collect()
    }
}

/// The topic name on a commit: None when it has no Topic trailer.
fn commit_topic(commit: &Commit) -> Result<Option<String>, PlanError> {
    let names = dedupe(commit.message.values("topic"));
    match names.as_slice() {
        [] => Ok(None),
        [name] if valid_topic_name(name) => Ok(Some(name.to_string())),
        [name] => Err(PlanError::BadTopicName {
            commit: short_sha(&commit.sha),
            name: name.to_string(),
        }),
        many => Err(PlanError::ManyTopics {
            commit: short_sha(&commit.sha),
            names: many.join(", "),
        }),
    }
}

/// Whether the commit has `Draft: true` (or `yes`). Every Draft value must be
/// one of `true`, `yes`, `false` or `no`, compared case-insensitively.
fn commit_draft(commit: &Commit) -> Result<bool, PlanError> {
    let mut draft = false;
    for value in commit.message.values("draft") {
        if value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("yes") {
            draft = true;
        } else if !(value.eq_ignore_ascii_case("false") || value.eq_ignore_ascii_case("no")) {
            return Err(PlanError::BadDraft {
                commit: short_sha(&commit.sha),
                value: value.to_string(),
            });
        }
    }
    Ok(draft)
}

/// The relative topic, reviewers and labels of one topic, checked against
/// the names of all topics.
fn topic_fields(
    topic: &Topic,
    names: &[String],
) -> Result<(Option<String>, Vec<String>, Vec<String>), PlanError> {
    let relatives = dedupe(
        topic
            .commits
            .iter()
            .flat_map(|commit| commit.message.values("relative")),
    );
    let relative = match relatives.as_slice() {
        [] => None,
        [only] if *only == topic.name.as_str() => {
            return Err(PlanError::SelfRelative {
                topic: topic.name.clone(),
            });
        }
        [only] if names.iter().any(|name| name.as_str() == *only) => Some(only.to_string()),
        [only] => {
            return Err(PlanError::UnknownRelative {
                topic: topic.name.clone(),
                relative: only.to_string(),
            });
        }
        many => {
            return Err(PlanError::ManyRelatives {
                topic: topic.name.clone(),
                relatives: many.join(", "),
            });
        }
    };
    Ok((
        relative,
        list_field(topic, "reviewers"),
        list_field(topic, "labels"),
    ))
}

/// The items of a list trailer across the topic's commits, split on commas
/// and whitespace, without duplicates, in first-seen order.
fn list_field(topic: &Topic, key: &str) -> Vec<String> {
    let items = topic
        .commits
        .iter()
        .flat_map(|commit| commit.message.values(key))
        .flat_map(split_items);
    dedupe(items).into_iter().map(String::from).collect()
}

fn split_items(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|item| !item.is_empty())
}

/// Orders topics so each comes after its relative. Repeatedly takes the
/// earliest remaining topic whose relative is None or already placed.
fn order(topics: Vec<Topic>) -> Result<Vec<Topic>, PlanError> {
    let mut remaining = topics;
    let mut ordered: Vec<Topic> = Vec::with_capacity(remaining.len());
    while !remaining.is_empty() {
        let ready = remaining.iter().position(|topic| match topic.relative.as_deref() {
            None => true,
            Some(relative) => ordered.iter().any(|placed| placed.name == relative),
        });
        let Some(index) = ready else {
            let stuck = remaining
                .iter()
                .map(|topic| format!("`{}`", topic.name))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(PlanError::Cycle { topics: stuck });
        };
        ordered.push(remaining.remove(index));
    }
    Ok(ordered)
}

/// The distinct items, in first-seen order.
fn dedupe<'a>(items: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let mut unique: Vec<&str> = Vec::new();
    for item in items {
        if !unique.contains(&item) {
            unique.push(item);
        }
    }
    unique
}

/// Topic names use only a-z, 0-9, `.`, `_` and `-`.
fn valid_topic_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// The first 8 characters of a sha, as errors show it.
fn short_sha(sha: &str) -> String {
    sha.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trailers::Message;

    fn commit(n: usize, raw: &str) -> Commit {
        Commit {
            sha: format!("{n:040}"),
            parent: format!("{:040}", n.wrapping_sub(1)),
            message: Message::parse(raw),
        }
    }

    fn topic_names(plan: &Plan) -> Vec<&str> {
        plan.topics.iter().map(|topic| topic.name.as_str()).collect()
    }

    fn subjects(topic: &Topic) -> Vec<&str> {
        topic
            .commits
            .iter()
            .map(|commit| commit.message.subject.as_str())
            .collect()
    }

    /// A chain base <- child <- grand, an unrelated topic `other`, and an
    /// untagged commit.
    fn stack() -> Plan {
        Plan::build(vec![
            commit(1, "Base\n\nTopic: base"),
            commit(2, "Child\n\nTopic: child\nRelative: base"),
            commit(3, "Grand\n\nTopic: grand\nRelative: child"),
            commit(4, "Other\n\nTopic: other"),
            commit(5, "Plain"),
        ])
        .expect("the stack builds")
    }

    #[test]
    fn untagged_commits_keep_branch_order() {
        let plan = Plan::build(vec![commit(1, "One"), commit(2, "Two")]).expect("builds");
        assert!(plan.topics.is_empty());
        let subjects: Vec<&str> = plan
            .untagged
            .iter()
            .map(|commit| commit.message.subject.as_str())
            .collect();
        assert_eq!(subjects, ["One", "Two"]);
    }

    #[test]
    fn non_adjacent_commits_group_into_one_topic_in_branch_order() {
        let plan = Plan::build(vec![
            commit(1, "A1\n\nTopic: alpha"),
            commit(2, "B1\n\nTopic: beta"),
            commit(3, "A2\n\nTopic: alpha"),
        ])
        .expect("builds");
        assert_eq!(topic_names(&plan), ["alpha", "beta"]);
        assert_eq!(subjects(&plan.topics[0]), ["A1", "A2"]);
        assert_eq!(subjects(&plan.topics[1]), ["B1"]);
        assert!(plan.untagged.is_empty());
    }

    #[test]
    fn a_relative_is_ordered_first_even_when_it_comes_later_in_the_branch() {
        let plan = Plan::build(vec![
            commit(1, "Child\n\nTopic: child\nRelative: base"),
            commit(2, "Base\n\nTopic: base"),
        ])
        .expect("builds");
        assert_eq!(topic_names(&plan), ["base", "child"]);
    }

    #[test]
    fn two_topics_on_one_commit_is_an_error() {
        let result = Plan::build(vec![commit(1, "A\n\nTopic: alpha\nTopic: beta")]);
        assert!(matches!(result, Err(PlanError::ManyTopics { .. })));
    }

    #[test]
    fn a_topic_name_with_capitals_is_an_error() {
        let result = Plan::build(vec![commit(1, "A\n\nTopic: Alpha")]);
        assert!(matches!(result, Err(PlanError::BadTopicName { .. })));
    }

    #[test]
    fn a_draft_value_other_than_yes_or_no_is_an_error() {
        let result = Plan::build(vec![commit(1, "A\n\nTopic: alpha\nDraft: maybe")]);
        assert!(matches!(result, Err(PlanError::BadDraft { .. })));
    }

    #[test]
    fn a_topic_with_two_relatives_is_an_error() {
        let result = Plan::build(vec![
            commit(1, "A\n\nTopic: alpha\nRelative: beta"),
            commit(2, "B\n\nTopic: alpha\nRelative: gamma"),
            commit(3, "C\n\nTopic: beta"),
            commit(4, "D\n\nTopic: gamma"),
        ]);
        assert!(matches!(result, Err(PlanError::ManyRelatives { .. })));
    }

    #[test]
    fn a_topic_relative_to_itself_is_an_error() {
        let result = Plan::build(vec![commit(1, "A\n\nTopic: alpha\nRelative: alpha")]);
        assert!(matches!(result, Err(PlanError::SelfRelative { .. })));
    }

    #[test]
    fn a_relative_that_no_commit_names_is_an_error() {
        let result = Plan::build(vec![commit(1, "A\n\nTopic: alpha\nRelative: missing")]);
        assert!(matches!(result, Err(PlanError::UnknownRelative { .. })));
    }

    #[test]
    fn topics_relative_to_each_other_in_a_loop_are_a_cycle() {
        let result = Plan::build(vec![
            commit(1, "A\n\nTopic: alpha\nRelative: beta"),
            commit(2, "B\n\nTopic: beta\nRelative: alpha"),
        ]);
        assert!(matches!(result, Err(PlanError::Cycle { .. })));
    }

    #[test]
    fn reviewers_and_labels_are_split_and_deduplicated_across_commits() {
        let plan = Plan::build(vec![
            commit(1, "A\n\nTopic: alpha\nReviewers: ana, bo\nLabels: ui, api"),
            commit(2, "B\n\nTopic: alpha\nReviewers: bo,  cy\tdi\nLabels: ui"),
        ])
        .expect("builds");
        let topic = plan.topic("alpha").expect("alpha exists");
        assert_eq!(topic.reviewers, ["ana", "bo", "cy", "di"]);
        assert_eq!(topic.labels, ["ui", "api"]);
    }

    #[test]
    fn draft_is_set_by_any_commit_and_compares_case_insensitively() {
        let plan = Plan::build(vec![
            commit(1, "A\n\nTopic: alpha\nDraft: no"),
            commit(2, "B\n\nTopic: alpha\nDraft: YES"),
            commit(3, "C\n\nTopic: beta\nDraft: False"),
        ])
        .expect("builds");
        assert!(plan.topic("alpha").is_some_and(|topic| topic.draft));
        assert!(plan.topic("beta").is_some_and(|topic| !topic.draft));
    }

    #[test]
    fn only_keeps_the_named_topic_and_its_ancestors_in_plan_order() {
        let plan = stack().only(&["grand".to_string()]).expect("grand exists");
        assert_eq!(topic_names(&plan), ["base", "child", "grand"]);
        assert_eq!(plan.untagged.len(), 1);
    }

    #[test]
    fn only_with_an_unknown_name_is_an_error() {
        let result = stack().only(&["grand".to_string(), "nope".to_string()]);
        assert!(matches!(result, Err(PlanError::UnknownTopic { .. })));
    }

    #[test]
    fn only_with_no_names_keeps_the_plan_unchanged() {
        let plan = stack();
        assert_eq!(plan.clone().only(&[]).expect("no names"), plan);
    }
}
