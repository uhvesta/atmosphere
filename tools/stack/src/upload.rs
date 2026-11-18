//! `stack upload`: pushes each topic's branch and opens or updates its pull
//! request, then writes the history comments and links any native stack.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;
use stack_graph::{Pr as GraphPr, Stack};

use crate::body::{self, Template};
use crate::cli::UploadArgs;
use crate::clock;
use crate::config::Config;
use crate::error::Error;
use crate::gh::{self, Gh, NewPr, PrEdit, PrState};
use crate::git::{Git, PushUpdate};
use crate::history::{self, History, Row, StackLine};
use crate::local::{self, Branches, Tip};
use crate::pins;
use crate::plan::{Plan, Topic};
use crate::restack;

/// What the upload did for a topic, or would do with `--dry-run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Created,
    Updated,
    Unchanged,
    Merged,
    WouldCreate,
    WouldUpdate,
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Action::Created => "created",
            Action::Updated => "updated",
            Action::Unchanged => "unchanged",
            Action::Merged => "merged",
            Action::WouldCreate => "would create",
            Action::WouldUpdate => "would update",
        }
    }
}

/// One topic's line in the result.
#[derive(Debug)]
struct Outcome {
    topic: String,
    branch: String,
    base: String,
    number: Option<u64>,
    url: Option<String>,
    action: Action,
}

/// The state every step of one run shares.
struct Session {
    git: Git,
    gh: Gh,
    remote: String,
    trunk: String,
    login: String,
    repo_url: String,
    branches: Branches,
    /// Whether to open new pull requests as drafts (`--draft` or stack.toml).
    draft: bool,
    /// Whether to replace descriptions edited on GitHub.
    overwrite_body: bool,
}

/// The pull requests already on GitHub for the plan's topics, by topic name.
struct Existing {
    open: BTreeMap<String, gh::Pr>,
    merged: BTreeMap<String, gh::Pr>,
}

/// A topic that is not merged, with what this run needs to know about it.
struct Item<'a> {
    topic: &'a Topic,
    branch: String,
    base: String,
    tip: Tip,
    /// The remote branch's head before this run; None when the branch is new.
    old_head: Option<String>,
    existing: Option<gh::Pr>,
}

impl Item<'_> {
    /// Whether the remote branch must move to the rebuilt tip.
    fn changed(&self) -> bool {
        self.old_head.as_deref() != Some(self.tip.head.as_str())
    }
}

/// A topic this run has put on GitHub.
struct Uploaded<'a> {
    topic: String,
    branch: String,
    base: String,
    number: u64,
    url: String,
    action: Action,
    subject: String,
    tip: Tip,
    /// The topic, for rendering its description again.
    source: &'a Topic,
    /// Its live description, as GitHub has it after this run's edits.
    body: String,
    /// Created, or its base was edited: its native stack chain gets linked.
    relinked: bool,
}

/// What happens to a pull request's description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyChange {
    /// The live description already matches.
    Same,
    /// It was edited on GitHub and `--overwrite-body` is not set: keep it.
    Kept,
    /// Replace it with the rendered description.
    Replace,
}

static TEMP_COUNT: AtomicU64 = AtomicU64::new(0);

/// A file holding text for gh to read, removed when dropped.
struct TempBody {
    path: PathBuf,
}

impl Drop for TempBody {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(crate) fn run(args: &UploadArgs, dir: &Path) -> Result<(), Error> {
    let git = Git::new(dir);
    restack::refuse_if_in_progress(&git)?;
    let config = Config::load()?;
    let template = Template::load(&config)?;
    let gh = Gh::new(dir);
    let repo = gh.repo()?;
    let login = gh.login()?;
    let prefix = config.prefix(&git, args.prefix.as_deref(), &login)?;
    let branches = Branches::new(prefix, gh.my_open_prs()?);
    let session = Session {
        trunk: config.trunk(&args.target, repo.default_branch.clone()),
        remote: config.remote(&args.target),
        git,
        gh,
        login,
        repo_url: repo.url,
        branches,
        draft: args.draft || config.draft.value,
        overwrite_body: args.overwrite_body || config.overwrite_body.value,
    };

    fetch_trunk_and_stack(&session)?;
    let (merge_base, plan) = local::read(&session.git, &session.remote, &session.trunk)?;
    let plan = plan.only(&args.topics)?;
    local::require_sections(&template, &plan)?;

    let mut existing = existing_prs(&session, &plan)?;
    let merged_names: BTreeSet<String> = existing.merged.keys().cloned().collect();
    let tips = local::rebuild(&session.git, &plan, &merge_base, &merged_names)?;
    let heads = local::remote_heads(&session.git, &session.remote, &session.branches.prefixes())?;
    let mut items = plan_items(&session, &plan, &tips, &heads, &mut existing);

    if args.dry_run {
        let graph = Stack::new(&existing_graph(&session, &items));
        let found: Vec<Outcome> = items
            .iter()
            .map(|item| dry_outcome(&session, item, &template, &graph))
            .collect();
        let outcomes = in_plan_order(&plan, &existing.merged, found);
        return print_outcomes(&outcomes, args.json, true);
    }

    retarget(&session, &mut items)?;
    let updates: Vec<PushUpdate> = items
        .iter()
        .filter(|item| item.changed())
        .map(|item| PushUpdate {
            branch: item.branch.clone(),
            new: item.tip.head.clone(),
            old: item.old_head.clone(),
        })
        .collect();
    session.git.push(&session.remote, &updates)?;

    let mut uploaded = write_prs(&session, &template, &items)?;
    let graph = Stack::new(&whole_stack(&session, &uploaded));
    write_stack_tables(&session, &template, &graph, &mut uploaded)?;
    let revisions = if config.history_comment.value {
        update_histories(&session, args.note.as_deref(), &graph, &uploaded)?
    } else {
        BTreeMap::new()
    };
    if config.pin_revisions.value {
        pin_revisions(&session, &uploaded, &revisions, config.keep_revisions.value)?;
    }
    if !args.no_native_stack && config.native_stack.value {
        link_native_stacks(&session, &graph, &uploaded);
    }

    let found: Vec<Outcome> = uploaded.iter().map(outcome_of).collect();
    let outcomes = in_plan_order(&plan, &existing.merged, found);
    print_outcomes(&outcomes, args.json, false)
}

/// Fetches the trunk and the stack branches of every prefix in use, so the
/// old heads are local when the patch ids are compared.
fn fetch_trunk_and_stack(session: &Session) -> Result<(), Error> {
    let (trunk, remote) = (&session.trunk, &session.remote);
    let mut refspecs = vec![format!("+refs/heads/{trunk}:refs/remotes/{remote}/{trunk}")];
    for prefix in session.branches.prefixes() {
        refspecs.push(format!("+refs/heads/{prefix}/stack/*:refs/remotes/{remote}/{prefix}/stack/*"));
    }
    session.git.fetch(remote, &refspecs)?;
    Ok(())
}

/// Looks up each topic's pull request. A closed one stops the upload before
/// anything is pushed; a merged one is left out of the rebuild.
fn existing_prs(session: &Session, plan: &Plan) -> Result<Existing, Error> {
    let mut found = Existing {
        open: BTreeMap::new(),
        merged: BTreeMap::new(),
    };
    for topic in &plan.topics {
        let pr = match session.branches.open_pr(&topic.name) {
            Some(pr) => pr.clone(),
            None => match session.gh.pr_for_head(&session.branches.branch(&topic.name))? {
                Some(pr) => pr,
                None => continue,
            },
        };
        match pr.state {
            PrState::Open => {
                found.open.insert(topic.name.clone(), pr);
            }
            PrState::Closed => {
                return Err(Error::ClosedPr {
                    topic: topic.name.clone(),
                    number: pr.number,
                });
            }
            PrState::Merged => {
                eprintln!(
                    "stack: note: topic `{}` (#{}) is merged; topics on it now target {}",
                    topic.name, pr.number, session.trunk
                );
                found.merged.insert(topic.name.clone(), pr);
            }
        }
    }
    Ok(found)
}

/// The topics that are not merged and have a rebuilt tip, in plan order.
fn plan_items<'a>(
    session: &Session,
    plan: &'a Plan,
    tips: &BTreeMap<String, Tip>,
    heads: &BTreeMap<String, String>,
    existing: &mut Existing,
) -> Vec<Item<'a>> {
    let mut items = Vec::new();
    for topic in &plan.topics {
        if existing.merged.contains_key(&topic.name) {
            continue;
        }
        let Some(tip) = tips.get(&topic.name) else {
            continue;
        };
        let branch = session.branches.branch(&topic.name);
        let base = match topic.relative.as_deref() {
            Some(relative) if !existing.merged.contains_key(relative) => session.branches.branch(relative),
            _ => session.trunk.clone(),
        };
        items.push(Item {
            topic,
            old_head: heads.get(&branch).cloned(),
            tip: tip.clone(),
            existing: existing.open.remove(&topic.name),
            branch,
            base,
        });
    }
    items
}

/// Moves each open pull request off a branch that is about to move: its base
/// goes to the trunk first, so GitHub doesn't see it as merged.
fn retarget(session: &Session, items: &mut [Item<'_>]) -> Result<(), Error> {
    for item in items.iter_mut() {
        let Some(pr) = item.existing.as_mut() else {
            continue;
        };
        if pr.base_ref_name == item.base || item.base == session.trunk {
            continue;
        }
        session.gh.edit_pr(&PrEdit {
            number: pr.number,
            base: Some(session.trunk.as_str()),
            title: None,
            body_file: None,
            add_reviewers: &[],
            add_labels: &[],
        })?;
        pr.base_ref_name = session.trunk.clone();
    }
    Ok(())
}

/// Creates or updates each topic's pull request, parents first. A new pull
/// request's description has no stack table yet: its number isn't known
/// until it exists, so `write_stack_tables` adds it.
fn write_prs<'a>(session: &Session, template: &Template, items: &[Item<'a>]) -> Result<Vec<Uploaded<'a>>, Error> {
    let mut uploaded = Vec::with_capacity(items.len());
    for item in items {
        let subject = subject_of(item.topic);
        let (number, url, action, relinked, body) = match &item.existing {
            None => {
                let description = template.render(item.topic, &session.trunk, &[]);
                let body_file = temp_body(&description)?;
                let (number, url) = session.gh.create_pr(&NewPr {
                    head: item.branch.as_str(),
                    base: item.base.as_str(),
                    title: subject,
                    body_file: body_file.path.as_path(),
                    draft: session.draft || item.topic.draft,
                    reviewers: item.topic.reviewers.as_slice(),
                    labels: item.topic.labels.as_slice(),
                })?;
                (number, url, Action::Created, true, description)
            }
            Some(pr) => {
                let (action, base_edited) = update_existing(&session.gh, item, pr, subject)?;
                (pr.number, pr.url.clone(), action, base_edited, pr.body.clone())
            }
        };
        uploaded.push(Uploaded {
            topic: item.topic.name.clone(),
            branch: item.branch.clone(),
            base: item.base.clone(),
            number,
            url,
            action,
            subject: subject.to_owned(),
            tip: item.tip.clone(),
            source: item.topic,
            body,
            relinked,
        });
    }
    Ok(uploaded)
}

/// Writes each description with its stack table, now every number is known.
/// A description edited on GitHub is kept unless `overwrite_body` is set.
fn write_stack_tables(
    session: &Session,
    template: &Template,
    graph: &Stack,
    uploaded: &mut [Uploaded<'_>],
) -> Result<(), Error> {
    for item in uploaded.iter_mut() {
        let description = template.render(item.source, &session.trunk, &stack_lines(graph, item.number));
        match body_change(&item.body, &description, session.overwrite_body) {
            BodyChange::Same => {}
            BodyChange::Kept => eprintln!(
                "stack: note: kept the description of #{}, which was edited on GitHub; pass --overwrite-body (or set overwrite_body in stack.toml) to replace it",
                item.number
            ),
            BodyChange::Replace => {
                let file = temp_body(&description)?;
                session.gh.edit_pr(&PrEdit {
                    number: item.number,
                    base: None,
                    title: None,
                    body_file: Some(file.path.as_path()),
                    add_reviewers: &[],
                    add_labels: &[],
                })?;
                item.body = description;
                if item.action == Action::Unchanged {
                    item.action = Action::Updated;
                }
            }
        }
    }
    Ok(())
}

/// The stack table rows for pull request `number`: its whole tree, with it
/// marked.
fn stack_lines(graph: &Stack, number: u64) -> Vec<StackLine> {
    graph
        .tree_of(number)
        .into_iter()
        .map(|node| StackLine {
            depth: node.depth,
            number: node.pr.number,
            title: node.pr.title.clone(),
            current: node.pr.number == number,
        })
        .collect()
}

/// Pins each uploaded head and its base under refs/stack/<prefix>/<topic>/,
/// as revision `revisions[topic]` (its history row), else one past the last
/// pin. A head already pinned at that revision is left alone; with `keep`
/// above 0, pins `keep` or more revisions older are deleted.
fn pin_revisions(
    session: &Session,
    uploaded: &[Uploaded<'_>],
    revisions: &BTreeMap<String, u32>,
    keep: u32,
) -> Result<(), Error> {
    let mut refs = Vec::new();
    for item in uploaded {
        let prefix = local::split_branch(&item.branch)
            .map_or_else(|| session.branches.prefix().to_owned(), |(prefix, _)| prefix.to_owned());
        let pinned = pins::on_remote(&session.git, &session.remote, &prefix, &item.topic)?;
        let rev = match revisions.get(&item.topic) {
            Some(rev) => *rev,
            None => match pinned.last_key_value() {
                Some((_, head)) if *head == item.tip.head => continue,
                Some((last, _)) => last + 1,
                None => 1,
            },
        };
        if pinned.get(&rev) == Some(&item.tip.head) {
            continue;
        }
        refs.extend(pins::updates(&prefix, &item.topic, rev, &item.tip.head, &item.tip.base, keep, &pinned));
    }
    session.git.push_refs(&session.remote, &refs)?;
    Ok(())
}

/// Edits an open pull request where its base or title differ, and its
/// reviewers and labels when the topic changed. Returns the action and
/// whether the base was edited. `write_stack_tables` does the description.
fn update_existing(gh: &Gh, item: &Item<'_>, pr: &gh::Pr, subject: &str) -> Result<(Action, bool), Error> {
    let base_edited = pr.base_ref_name != item.base;
    let title_edited = pr.title != subject;
    let none: &[String] = &[];
    let (reviewers, labels) = if item.changed() {
        (item.topic.reviewers.as_slice(), item.topic.labels.as_slice())
    } else {
        (none, none)
    };
    gh.edit_pr(&PrEdit {
        number: pr.number,
        base: base_edited.then_some(item.base.as_str()),
        title: title_edited.then_some(subject),
        body_file: None,
        add_reviewers: reviewers,
        add_labels: labels,
    })?;
    let edited = base_edited || title_edited;
    let action = if item.changed() || edited {
        Action::Updated
    } else {
        Action::Unchanged
    };
    Ok((action, base_edited))
}

/// Writes each uploaded topic's history comment: a row when its head moved,
/// and the stack tree. Creates the comment when there is none. Returns each
/// topic's revision: the number of the row for its current head.
fn update_histories(
    session: &Session,
    note: Option<&str>,
    graph: &Stack,
    uploaded: &[Uploaded<'_>],
) -> Result<BTreeMap<String, u32>, Error> {
    let mut revisions = BTreeMap::new();
    for item in uploaded {
        let lines = stack_lines(graph, item.number);
        let comments = session.gh.comments(item.number)?;
        let found = history::find(&comments, &session.login);
        let mut hist = found
            .map(|comment| History::parse(&comment.body))
            .unwrap_or_default();

        if hist.rows.last().map(|row| row.head.as_str()) != Some(item.tip.head.as_str()) {
            let same = same_patch(&session.git, hist.rows.last(), &item.tip)?;
            let diff = history::classify(hist.rows.last(), &item.tip.base, same);
            let mut summary = session
                .git
                .shortstat(&item.tip.base, &item.tip.head)?;
            if let Some(note) = note {
                summary = format!("{summary} · {note}");
            }
            let rev = hist.next_rev();
            hist.rows.push(Row {
                rev,
                head: item.tip.head.clone(),
                base: item.tip.base.clone(),
                diff,
                pushed: clock::now_utc(),
                summary,
            });
        }

        if let Some(row) = hist.rows.last() {
            revisions.insert(item.topic.clone(), row.rev);
        }
        let text = history::render(&session.repo_url, &session.trunk, &item.topic, &lines, &hist);
        match found {
            Some(comment) if normalised(&comment.body) == normalised(&text) => {}
            Some(comment) => {
                let file = temp_body(&text)?;
                session.gh.edit_comment(comment.id, &file.path)?;
            }
            None => {
                let file = temp_body(&text)?;
                session.gh.add_comment(item.number, &file.path)?;
            }
        }
    }
    Ok(revisions)
}

/// Whether the last row's patch equals the tip's patch. False when there is
/// no last row, or its commits are no longer in the checkout.
fn same_patch(git: &Git, last: Option<&Row>, tip: &Tip) -> Result<bool, Error> {
    let Some(last) = last else {
        return Ok(false);
    };
    if !(git.has_commit(&last.base) && git.has_commit(&last.head)) {
        return Ok(false);
    }
    let before = git.patch_id(&last.base, &last.head)?;
    let after = git.patch_id(&tip.base, &tip.head)?;
    Ok(before == after)
}

/// Links each native chain that holds a pull request created or re-based in
/// this run. A refusal is a note: the pull requests stay chained by base.
fn link_native_stacks(session: &Session, graph: &Stack, uploaded: &[Uploaded<'_>]) {
    let chains: Vec<Vec<u64>> = graph
        .native_chains()
        .into_iter()
        .filter(|chain| {
            chain.iter().any(|number| {
                uploaded
                    .iter()
                    .any(|item| item.relinked && item.number == *number)
            })
        })
        .collect();
    if chains.is_empty() || !session.gh.has_native_stacks() {
        return;
    }
    for chain in chains {
        let urls: Vec<String> = chain
            .iter()
            .filter_map(|number| {
                uploaded
                    .iter()
                    .find(|item| item.number == *number)
                    .map(|item| item.url.clone())
                    .or_else(|| {
                        session
                            .branches
                            .open_prs()
                            .find(|pr| pr.number == *number)
                            .map(|pr| pr.url.clone())
                    })
            })
            .collect();
        if let Err(error) = session.gh.link_stack(&session.trunk, &urls) {
            let numbers: Vec<String> = chain.iter().map(|number| format!("#{number}")).collect();
            eprintln!(
                "stack: note: could not link {} as a native stack ({error}); the pull requests are still chained by base",
                numbers.join(", ")
            );
        }
    }
}

/// The outcome for each topic a dry run would touch, with no writes. `graph`
/// holds the pull requests that exist already.
fn dry_outcome(session: &Session, item: &Item<'_>, template: &Template, graph: &Stack) -> Outcome {
    let overwrite = session.overwrite_body;
    let (action, number, url) = match &item.existing {
        None => (Action::WouldCreate, None, None),
        Some(pr) => {
            let description = template.render(item.topic, &session.trunk, &stack_lines(graph, pr.number));
            let edits = item.changed()
                || pr.base_ref_name != item.base
                || pr.title != subject_of(item.topic)
                || body_change(&pr.body, &description, overwrite) == BodyChange::Replace;
            let action = if edits {
                Action::WouldUpdate
            } else {
                Action::Unchanged
            };
            (action, Some(pr.number), Some(pr.url.clone()))
        }
    };
    Outcome {
        topic: item.topic.name.clone(),
        branch: item.branch.clone(),
        base: item.base.clone(),
        number,
        url,
        action,
    }
}

/// The outcomes in plan order, merged topics included. `found` holds the
/// topics this run handled; the rest are merged.
fn in_plan_order(plan: &Plan, merged: &BTreeMap<String, gh::Pr>, found: Vec<Outcome>) -> Vec<Outcome> {
    let mut by_topic: BTreeMap<String, Outcome> = found
        .into_iter()
        .map(|outcome| (outcome.topic.clone(), outcome))
        .collect();
    plan.topics
        .iter()
        .filter_map(|topic| {
            by_topic.remove(&topic.name).or_else(|| {
                merged
                    .get(&topic.name)
                    .map(|pr| merged_outcome(&topic.name, pr))
            })
        })
        .collect()
}

fn merged_outcome(topic: &str, pr: &gh::Pr) -> Outcome {
    Outcome {
        topic: topic.to_owned(),
        branch: pr.head_ref_name.clone(),
        base: pr.base_ref_name.clone(),
        number: Some(pr.number),
        url: Some(pr.url.clone()),
        action: Action::Merged,
    }
}

fn outcome_of(item: &Uploaded<'_>) -> Outcome {
    Outcome {
        topic: item.topic.clone(),
        branch: item.branch.clone(),
        base: item.base.clone(),
        number: Some(item.number),
        url: Some(item.url.clone()),
        action: item.action,
    }
}

fn print_outcomes(outcomes: &[Outcome], json: bool, dry_run: bool) -> Result<(), Error> {
    if json {
        let array: Vec<Value> = outcomes.iter().map(outcome_json).collect();
        let text = serde_json::to_string_pretty(&Value::Array(array))
            .map_err(|source| Error::Json { source })?;
        println!("{text}");
        return Ok(());
    }
    for outcome in outcomes {
        if dry_run {
            println!("{}", dry_line(outcome));
        } else {
            println!("{}", done_line(outcome));
        }
    }
    Ok(())
}

/// `<topic>  <branch> -> <base>  would create a PR`, or `... would update #n`.
fn dry_line(outcome: &Outcome) -> String {
    let what = match (outcome.action, outcome.number) {
        (Action::WouldCreate, _) => "would create a PR".to_owned(),
        (action, Some(number)) => format!("{} #{number}", action.label()),
        (action, None) => action.label().to_owned(),
    };
    format!(
        "{}  {} -> {}  {}",
        outcome.topic, outcome.branch, outcome.base, what
    )
}

/// `<topic>  #<n> <action>  <url>`.
fn done_line(outcome: &Outcome) -> String {
    let number = outcome
        .number
        .map(|number| format!("#{number}"))
        .unwrap_or_default();
    let url = outcome.url.as_deref().unwrap_or_default();
    format!(
        "{}  {} {}  {}",
        outcome.topic,
        number,
        outcome.action.label(),
        url
    )
}

fn outcome_json(outcome: &Outcome) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("topic".to_owned(), Value::from(outcome.topic.as_str()));
    object.insert("branch".to_owned(), Value::from(outcome.branch.as_str()));
    object.insert("base".to_owned(), Value::from(outcome.base.as_str()));
    object.insert("number".to_owned(), Value::from(outcome.number));
    object.insert("url".to_owned(), Value::from(outcome.url.clone()));
    object.insert("action".to_owned(), Value::from(outcome.action.label()));
    Value::Object(object)
}

/// The pull requests that exist before this run, for a dry run's stack
/// tables: every open stack pull request, with this run's topics on the
/// bases they would get.
fn existing_graph(session: &Session, items: &[Item<'_>]) -> Vec<GraphPr> {
    let mut prs: BTreeMap<u64, GraphPr> = session
        .branches
        .open_prs()
        .map(|pr| {
            (
                pr.number,
                GraphPr {
                    number: pr.number,
                    head: pr.head_ref_name.clone(),
                    base: pr.base_ref_name.clone(),
                    title: pr.title.clone(),
                    from_fork: false,
                },
            )
        })
        .collect();
    for item in items {
        if let Some(pr) = &item.existing {
            prs.insert(
                pr.number,
                GraphPr {
                    number: pr.number,
                    head: item.branch.clone(),
                    base: item.base.clone(),
                    title: subject_of(item.topic).to_owned(),
                    from_fork: false,
                },
            );
        }
    }
    prs.into_values().collect()
}

/// Every open stack pull request as GitHub had it at the start of the run,
/// with this run's pull requests as they are now, so a table written by
/// `upload <topic>` still shows the topics this run didn't touch.
fn whole_stack(session: &Session, uploaded: &[Uploaded<'_>]) -> Vec<GraphPr> {
    let mut prs: BTreeMap<u64, GraphPr> = session
        .branches
        .open_prs()
        .map(|pr| {
            (
                pr.number,
                GraphPr {
                    number: pr.number,
                    head: pr.head_ref_name.clone(),
                    base: pr.base_ref_name.clone(),
                    title: pr.title.clone(),
                    from_fork: false,
                },
            )
        })
        .collect();
    for item in uploaded {
        prs.insert(
            item.number,
            GraphPr {
                number: item.number,
                head: item.branch.clone(),
                base: item.base.clone(),
                title: item.subject.clone(),
                from_fork: false,
            },
        );
    }
    prs.into_values().collect()
}

/// The first commit's subject: the pull request's title.
fn subject_of(topic: &Topic) -> &str {
    topic
        .commits
        .first()
        .map_or("", |commit| commit.message.subject.as_str())
}

/// Compares a live description with the rendered one. A live description
/// that stack didn't write (its marker doesn't match) counts as edited.
fn body_change(live: &str, description: &str, overwrite: bool) -> BodyChange {
    if normalised(live) == normalised(description) {
        BodyChange::Same
    } else if body::edited_since_upload(live) && !overwrite {
        BodyChange::Kept
    } else {
        BodyChange::Replace
    }
}

/// The text as stack compares descriptions and comments: without `\r`, and
/// without surrounding whitespace.
fn normalised(text: &str) -> String {
    text.replace('\r', "").trim().to_owned()
}

/// Writes `text` to a new file in the temp folder for gh to read. The file
/// is removed when the returned value is dropped.
fn temp_body(text: &str) -> Result<TempBody, Error> {
    let count = TEMP_COUNT.fetch_add(1, Ordering::Relaxed);
    let file = TempBody {
        path: std::env::temp_dir().join(format!("stack-{}-{count}.md", std::process::id())),
    };
    std::fs::write(&file.path, text).map_err(|source| Error::TempFile {
        path: file.path.clone(),
        source,
    })?;
    Ok(file)
}
