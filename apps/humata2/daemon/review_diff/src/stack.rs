//! A stack of layers (RV-36, D800): a review that steps through a
//! change, bottom first, across repos. [`Stack::check_blocking`] refuses
//! what can't be shown and resolves every rev to a commit; the
//! [`CheckedStack`] makes one [`DiffSpec`] per layer and for the whole
//! stack, and reports [`Drift`] once a kept rev has moved.

use std::collections::BTreeSet;

use thiserror::Error;

use crate::plan::{FileRef, Group, Importance, Tier};
use crate::spec::{Base, Compare, DiffSpec, RepoSpec, Whitespace};
use crate::{Checkout, Clones, DiffError};

const MAX_LAYERS: usize = 50;
const MAX_TITLE: usize = 60;
const MAX_PURPOSE: usize = 120;

/// A stack of layers, bottom first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stack {
    pub layers: Vec<Layer>,
    pub whitespace: Whitespace,
    pub renames: bool,
}

/// One step of a stack.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layer {
    /// The layer's identity: unique ignoring case, at most 60 characters.
    pub title: String,
    /// One line, at most 120 characters.
    pub purpose: Option<String>,
    /// The repos the layer touches, each once.
    pub repos: Vec<LayerRepo>,
    /// The stack topic the layer was read from (D803), which is then its
    /// identity for drift and refresh.
    pub topic: Option<String>,
    /// This layer's reading order and groups (D802); a picked layer uses
    /// these and nothing from the plan.
    pub order: Vec<FileRef>,
    pub groups: Vec<Group>,
    /// The files the agent marked key or skim in this layer (D802).
    pub importance: Vec<Importance>,
}

/// The order, groups and importance a view of the stack reads its files by
/// (D802).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilePlan {
    pub order: Vec<FileRef>,
    pub groups: Vec<Group>,
    pub importance: Vec<Importance>,
}

/// A layer's part in one repo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerRepo {
    pub repo: String,
    pub part: Part,
    /// The pull request that carries this part, for display.
    pub pull_request: Option<String>,
}

/// What a layer shows in a repo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    /// The commits after `base` up to `head`, revs as sent.
    Range { base: String, head: String },
    /// HEAD plus the working-tree files under `include_paths`.
    Worktree { include_paths: Vec<String> },
}

/// The commits a layer's part resolved to when the stack was checked. A
/// worktree part has HEAD's commit as both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub base_commit: String,
    pub head_commit: String,
}

/// Why a stack can't be shown.
#[derive(Debug, Error)]
pub enum StackError {
    #[error("the stack has no layers; send at least one layer, or send a spec instead of a stack")]
    NoLayers,

    #[error(
        "the stack has {count} layers and at most {max} are allowed; group commits into fewer layers and send it again"
    )]
    TooManyLayers { count: usize, max: usize },

    #[error("layer {position} has a bad title: {reason}; give every layer a short title and send it again")]
    BadTitle { position: usize, reason: String },

    #[error(
        "two layers are titled `{title}` (titles are compared ignoring case and spaces); give each layer its own title"
    )]
    DuplicateTitle { title: String },

    #[error("layer `{layer}` has a bad purpose: {reason}; send one short line or leave it out")]
    BadPurpose { layer: String, reason: String },

    #[error("layer `{layer}` touches no repo; list the repos it changes, or drop the layer")]
    EmptyLayer { layer: String },

    #[error("layer `{layer}` lists repo `{repo}` twice; list each repo once per layer")]
    RepoTwice { layer: String, repo: String },

    #[error(
        "layer `{layer}` names repo `{repo}`, which this session doesn't have (it has: {}); use one of those names",
        known.join(", ")
    )]
    UnknownRepo {
        layer: String,
        repo: String,
        known: Vec<String>,
    },

    #[error(
        "layer `{layer}` in `{repo}` is uncommitted work with no paths; list the paths it covers, or \".\" for everything"
    )]
    NoIncludePaths { layer: String, repo: String },

    #[error(
        "layer `{layer}` in `{repo}` names `{rev}`, which isn't in the clone; fetch it or send a rev the clone has"
    )]
    UnknownRevision {
        layer: String,
        repo: String,
        rev: String,
    },

    #[error(
        "layer `{layer}` in `{repo}`: `{head}` is not a descendant of `{base}`; send the layer's base as an ancestor of its head"
    )]
    NotAncestor {
        layer: String,
        repo: String,
        base: String,
        head: String,
    },

    #[error(
        "layer `{layer}` in `{repo}` starts at `{base}`, but the layer below ends at `{expected}`; send the layer below's head as its base (fetch it if it is missing), or send the other branch as its own review"
    )]
    ChainBreak {
        layer: String,
        repo: String,
        expected: String,
        base: String,
    },

    #[error(
        "layer `{layer}` in `{repo}` is uncommitted work below a commit range; put uncommitted layers above the commits, or send the uncommitted work as its own review"
    )]
    WorktreeBelowRange { layer: String, repo: String },

    #[error(
        "layer `{layer}` in `{repo}` is uncommitted work, but `{repo}` has `{head}` checked out and not the top of the stack; check out the top head, or send the uncommitted work as its own review"
    )]
    HeadNotAtTop {
        layer: String,
        repo: String,
        head: String,
    },

    #[error("there is no layer `{title}` (the layers are: {}); use one of those titles", known.join(", "))]
    UnknownLayer { title: String, known: Vec<String> },

    #[error("git failed for layer `{layer}` in `{repo}`; check that the clone is healthy and send the stack again")]
    Git {
        layer: String,
        repo: String,
        #[source]
        source: DiffError,
    },
}

impl Stack {
    /// A stack with git's defaults: whitespace counts, renames detected.
    #[must_use]
    pub fn new(layers: Vec<Layer>) -> Self {
        Self {
            layers,
            whitespace: Whitespace::Counts,
            renames: true,
        }
    }

    /// Checks the stack against the clones and resolves its revs.
    ///
    /// # Errors
    ///
    /// The [`StackError`] naming the first layer and repo that can't be
    /// shown.
    pub fn check_blocking(&self, clones: &Clones) -> Result<CheckedStack, StackError> {
        self.check_shape(clones)?;
        let mut resolved: Vec<Vec<Option<Resolved>>> = self
            .layers
            .iter()
            .map(|layer| vec![None; layer.repos.len()])
            .collect();
        for repo in self.repo_names() {
            // The shape check saw every repo has a clone.
            let Ok(checkout) = clones.get(&repo) else {
                continue;
            };
            self.check_repo(checkout, &repo, &mut resolved)?;
        }
        let resolved = resolved
            .into_iter()
            .map(|row| row.into_iter().flatten().collect())
            .collect();
        Ok(CheckedStack {
            stack: self.clone(),
            resolved,
        })
    }

    /// Repos in first-appearance order.
    fn repo_names(&self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut names = Vec::new();
        for part in self.layers.iter().flat_map(|layer| &layer.repos) {
            if seen.insert(part.repo.as_str()) {
                names.push(part.repo.clone());
            }
        }
        names
    }

    fn check_shape(&self, clones: &Clones) -> Result<(), StackError> {
        if self.layers.is_empty() {
            return Err(StackError::NoLayers);
        }
        if self.layers.len() > MAX_LAYERS {
            return Err(StackError::TooManyLayers {
                count: self.layers.len(),
                max: MAX_LAYERS,
            });
        }
        let mut titles = BTreeSet::new();
        for (index, layer) in self.layers.iter().enumerate() {
            let title = layer.title.trim();
            if title.is_empty() {
                return Err(StackError::BadTitle {
                    position: index + 1,
                    reason: "it is empty".to_owned(),
                });
            }
            if title.chars().count() > MAX_TITLE {
                return Err(StackError::BadTitle {
                    position: index + 1,
                    reason: format!("it is longer than {MAX_TITLE} characters"),
                });
            }
            if !titles.insert(title.to_lowercase()) {
                return Err(StackError::DuplicateTitle {
                    title: title.to_owned(),
                });
            }
            if let Some(purpose) = &layer.purpose {
                let reason = if purpose.contains('\n') || purpose.contains('\r') {
                    Some("it has more than one line".to_owned())
                } else if purpose.chars().count() > MAX_PURPOSE {
                    Some(format!("it is longer than {MAX_PURPOSE} characters"))
                } else {
                    None
                };
                if let Some(reason) = reason {
                    return Err(StackError::BadPurpose {
                        layer: title.to_owned(),
                        reason,
                    });
                }
            }
            if layer.repos.is_empty() {
                return Err(StackError::EmptyLayer {
                    layer: title.to_owned(),
                });
            }
            let mut repos = BTreeSet::new();
            for part in &layer.repos {
                if !repos.insert(part.repo.as_str()) {
                    return Err(StackError::RepoTwice {
                        layer: title.to_owned(),
                        repo: part.repo.clone(),
                    });
                }
                if let Err(error) = clones.get(&part.repo) {
                    let known = match error {
                        DiffError::UnknownRepo { known, .. } => known,
                        _ => Vec::new(),
                    };
                    return Err(StackError::UnknownRepo {
                        layer: title.to_owned(),
                        repo: part.repo.clone(),
                        known,
                    });
                }
                if matches!(&part.part, Part::Worktree { include_paths } if include_paths.is_empty())
                {
                    return Err(StackError::NoIncludePaths {
                        layer: title.to_owned(),
                        repo: part.repo.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Walks `repo`'s parts bottom up, filling `resolved`.
    fn check_repo(
        &self,
        checkout: &Checkout,
        repo: &str,
        resolved: &mut [Vec<Option<Resolved>>],
    ) -> Result<(), StackError> {
        let mut top_head: Option<(String, String)> = None; // (rev, commit)
        let mut worktree: Option<(String, String)> = None; // (layer, HEAD commit)
        for (index, layer) in self.layers.iter().enumerate() {
            let title = layer.title.trim();
            let Some((slot, part)) = layer
                .repos
                .iter()
                .enumerate()
                .find(|(_, part)| part.repo == repo)
            else {
                continue;
            };
            let git = |source| StackError::Git {
                layer: title.to_owned(),
                repo: repo.to_owned(),
                source,
            };
            let value = match &part.part {
                Part::Range { base, head } => {
                    if let Some((worktree_layer, _)) = &worktree {
                        return Err(StackError::WorktreeBelowRange {
                            layer: worktree_layer.clone(),
                            repo: repo.to_owned(),
                        });
                    }
                    let base_commit = commit_of(checkout, title, repo, base)?;
                    let head_commit = commit_of(checkout, title, repo, head)?;
                    if !is_ancestor(checkout, &base_commit, &head_commit).map_err(git)? {
                        return Err(StackError::NotAncestor {
                            layer: title.to_owned(),
                            repo: repo.to_owned(),
                            base: base.clone(),
                            head: head.clone(),
                        });
                    }
                    if let Some((below_rev, below_commit)) = &top_head {
                        if *below_commit != base_commit {
                            return Err(StackError::ChainBreak {
                                layer: title.to_owned(),
                                repo: repo.to_owned(),
                                expected: below_rev.clone(),
                                base: base.clone(),
                            });
                        }
                    }
                    top_head = Some((head.clone(), head_commit.clone()));
                    Resolved {
                        base_commit,
                        head_commit,
                    }
                }
                Part::Worktree { .. } => {
                    let head = commit_of(checkout, title, repo, "HEAD")?;
                    if worktree.is_none() {
                        worktree = Some((title.to_owned(), head.clone()));
                    }
                    Resolved {
                        base_commit: head.clone(),
                        head_commit: head,
                    }
                }
            };
            if let Some(cell) = resolved.get_mut(index).and_then(|row| row.get_mut(slot)) {
                *cell = Some(value);
            }
        }
        if let (Some((layer, head)), Some((_, top))) = (&worktree, &top_head) {
            if head != top {
                return Err(StackError::HeadNotAtTop {
                    layer: layer.clone(),
                    repo: repo.to_owned(),
                    head: short(head),
                });
            }
        }
        Ok(())
    }

    /// The index of the layer titled `title`, ignoring case and spaces at
    /// the ends.
    fn index_of(&self, title: &str) -> Option<usize> {
        let wanted = title.trim().to_lowercase();
        self.layers
            .iter()
            .position(|layer| layer.title.trim().to_lowercase() == wanted)
    }

    /// What a picked layer reads by: its own order, groups and importance,
    /// and nothing from the plan (D802).
    ///
    /// # Errors
    ///
    /// [`StackError::UnknownLayer`] when no layer has that title.
    pub fn layer_plan(&self, title: &str) -> Result<FilePlan, StackError> {
        let layer = self
            .index_of(title)
            .and_then(|index| self.layers.get(index))
            .ok_or_else(|| StackError::UnknownLayer {
                title: title.trim().to_owned(),
                known: self
                    .layers
                    .iter()
                    .map(|layer| layer.title.trim().to_owned())
                    .collect(),
            })?;
        Ok(FilePlan {
            order: layer.order.clone(),
            groups: layer.groups.clone(),
            importance: layer.importance.clone(),
        })
    }

    /// What Whole stack reads by when the plan sent no order, groups or
    /// importance (D802): the layers' orders joined bottom first with each
    /// file at its first appearance, no groups, and per file key if any
    /// layer says key, skim only if every layer touching it says skim. Only
    /// key and skim files are listed.
    #[must_use]
    pub fn whole_plan(&self) -> FilePlan {
        let mut order: Vec<FileRef> = Vec::new();
        let mut seen = BTreeSet::new();
        // Files in first-appearance order, with the tier each touching
        // layer gave.
        let mut touched: Vec<FileRef> = Vec::new();
        let mut tiers: std::collections::BTreeMap<FileRef, Vec<Tier>> = std::collections::BTreeMap::new();
        for layer in &self.layers {
            let mut here: std::collections::BTreeMap<&FileRef, Tier> = std::collections::BTreeMap::new();
            for file in &layer.order {
                here.entry(file).or_default();
                if seen.insert(file.clone()) {
                    order.push(file.clone());
                }
            }
            for entry in &layer.importance {
                here.insert(&entry.file, entry.tier);
            }
            for file in layer
                .order
                .iter()
                .chain(layer.importance.iter().map(|entry| &entry.file))
            {
                if let Some(tier) = here.remove(file) {
                    if !tiers.contains_key(file) {
                        touched.push(file.clone());
                    }
                    tiers.entry(file.clone()).or_default().push(tier);
                }
            }
        }
        let importance = touched
            .into_iter()
            .filter_map(|file| {
                let given = tiers.get(&file)?;
                let tier = if given.contains(&Tier::Key) {
                    Tier::Key
                } else if given.iter().all(|tier| *tier == Tier::Skim) {
                    Tier::Skim
                } else {
                    return None;
                };
                Some(Importance { file, tier })
            })
            .collect();
        FilePlan {
            order,
            groups: Vec::new(),
            importance,
        }
    }
}

fn short(commit: &str) -> String {
    commit.chars().take(12).collect()
}

fn commit_of(
    checkout: &Checkout,
    layer: &str,
    repo: &str,
    rev: &str,
) -> Result<String, StackError> {
    checkout.commit(rev).map_err(|source| match source {
        DiffError::UnknownRevision { .. } => StackError::UnknownRevision {
            layer: layer.to_owned(),
            repo: repo.to_owned(),
            rev: rev.to_owned(),
        },
        source => StackError::Git {
            layer: layer.to_owned(),
            repo: repo.to_owned(),
            source,
        },
    })
}

/// Whether `base` is `head` or one of its ancestors.
fn is_ancestor(checkout: &Checkout, base: &str, head: &str) -> Result<bool, DiffError> {
    let args = ["merge-base", "--is-ancestor", base, head];
    let output = checkout.git.output(args, None)?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(checkout.git.failed(args, &output)),
    }
}

/// A [`Stack`] that passed its check, with the commits each part resolved
/// to.
#[derive(Clone, Debug)]
pub struct CheckedStack {
    stack: Stack,
    /// Per layer, per repo of the layer in its order.
    resolved: Vec<Vec<Resolved>>,
}

/// Which part of a layer drifted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drift {
    /// The layer's title as sent.
    pub layer: String,
    pub repo: String,
    pub kind: DriftKind,
}

/// How a kept rev stopped holding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriftKind {
    /// A rev now sits elsewhere and ancestry or the chain no longer holds.
    Moved,
    /// A rev no longer resolves.
    Missing,
    /// HEAD left the top head an uncommitted layer sits on.
    HeadMoved,
}

impl CheckedStack {
    /// The stack as sent.
    #[must_use]
    pub fn stack(&self) -> &Stack {
        &self.stack
    }

    /// What `layer` (by title) resolved to in `repo` when checked.
    #[must_use]
    pub fn resolved(&self, layer: &str, repo: &str) -> Option<&Resolved> {
        let index = self.index_of(layer)?;
        let slot = self
            .stack
            .layers
            .get(index)?
            .repos
            .iter()
            .position(|part| part.repo == repo)?;
        self.resolved.get(index)?.get(slot)
    }

    fn index_of(&self, title: &str) -> Option<usize> {
        self.stack.index_of(title)
    }

    fn repo_spec(&self, repo: &str, base: Base, compare: Compare) -> RepoSpec {
        let mut spec = RepoSpec::new(repo, base, compare);
        spec.whitespace = self.stack.whitespace;
        spec.renames = self.stack.renames;
        spec
    }

    /// The diff of one layer, over only the repos it touches.
    ///
    /// # Errors
    ///
    /// [`StackError::UnknownLayer`] when no layer has that title.
    pub fn layer_spec(&self, title: &str) -> Result<DiffSpec, StackError> {
        let Some(index) = self.index_of(title) else {
            return Err(StackError::UnknownLayer {
                title: title.trim().to_owned(),
                known: self
                    .stack
                    .layers
                    .iter()
                    .map(|layer| layer.title.trim().to_owned())
                    .collect(),
            });
        };
        let (Some(layer), Some(resolved)) =
            (self.stack.layers.get(index), self.resolved.get(index))
        else {
            return Ok(DiffSpec::default());
        };
        let repos = layer
            .repos
            .iter()
            .zip(resolved)
            .map(|(part, resolved)| match &part.part {
                Part::Range { base, head } => self.repo_spec(
                    &part.repo,
                    Base::Rev { rev: base.clone() },
                    Compare::at(head.clone()),
                ),
                Part::Worktree { include_paths } => self.repo_spec(
                    &part.repo,
                    Base::Rev {
                        rev: resolved.head_commit.clone(),
                    },
                    Compare::worktree(include_paths.clone()),
                ),
            })
            .collect();
        Ok(DiffSpec { repos })
    }

    /// What a picked layer reads by: its own order, groups and importance,
    /// and nothing from the plan (D802). As [`Stack::layer_plan`].
    ///
    /// # Errors
    ///
    /// [`StackError::UnknownLayer`] when no layer has that title.
    pub fn layer_plan(&self, title: &str) -> Result<FilePlan, StackError> {
        self.stack.layer_plan(title)
    }

    /// What Whole stack reads by when the plan sent none (D802). As
    /// [`Stack::whole_plan`].
    #[must_use]
    pub fn whole_plan(&self) -> FilePlan {
        self.stack.whole_plan()
    }

    /// The whole stack: per repo, from the first base to the top head, plus
    /// the uncommitted work under the worktree layers' paths.
    #[must_use]
    pub fn whole(&self) -> DiffSpec {
        let mut repos = Vec::new();
        for repo in self.stack.repo_names() {
            let parts: Vec<(&Part, &Resolved)> = self
                .stack
                .layers
                .iter()
                .zip(&self.resolved)
                .flat_map(|(layer, resolved)| {
                    layer
                        .repos
                        .iter()
                        .zip(resolved)
                        .filter(|(part, _)| part.repo == repo)
                        .map(|(part, resolved)| (&part.part, resolved))
                })
                .collect();
            let Some((first, first_resolved)) = parts.first() else {
                continue;
            };
            let base = match first {
                Part::Range { base, .. } => base.clone(),
                Part::Worktree { .. } => first_resolved.head_commit.clone(),
            };
            let mut paths: Vec<String> = Vec::new();
            let mut top: Option<String> = None;
            for (part, _) in &parts {
                match part {
                    Part::Range { head, .. } => top = Some(head.clone()),
                    Part::Worktree { include_paths } => {
                        for path in include_paths {
                            if !paths.contains(path) {
                                paths.push(path.clone());
                            }
                        }
                    }
                }
            }
            let compare = if paths.is_empty() {
                match top {
                    Some(head) => Compare::at(head),
                    None => Compare::commits(),
                }
            } else {
                Compare::worktree(paths)
            };
            repos.push(self.repo_spec(&repo, Base::Rev { rev: base }, compare));
        }
        DiffSpec { repos }
    }

    /// Re-resolves every rev and reports what no longer holds, naming layers
    /// as sent. A rev that moved but still chains is not drift.
    #[must_use]
    pub fn drift_blocking(&self, clones: &Clones) -> Vec<Drift> {
        let mut drift = Vec::new();
        for repo in self.stack.repo_names() {
            let checkout = clones.get(&repo).ok();
            // The previous range layer's current head commit, once known.
            let mut below: Option<String> = None;
            let mut top: Option<String> = None;
            let mut report = |layer: &Layer, kind| {
                drift.push(Drift {
                    layer: layer.title.clone(),
                    repo: repo.clone(),
                    kind,
                });
            };
            for (layer, resolved) in self.stack.layers.iter().zip(&self.resolved) {
                let Some((part, kept)) = layer
                    .repos
                    .iter()
                    .zip(resolved)
                    .find(|(part, _)| part.repo == repo)
                else {
                    continue;
                };
                match &part.part {
                    Part::Range { base, head } => {
                        let now = checkout.and_then(|checkout| {
                            let base = checkout.commit(base).ok()?;
                            let head = checkout.commit(head).ok()?;
                            Some((checkout, base, head))
                        });
                        let Some((checkout, base_now, head_now)) = now else {
                            below = None;
                            top = None;
                            report(layer, DriftKind::Missing);
                            continue;
                        };
                        let moved = base_now != kept.base_commit || head_now != kept.head_commit;
                        let chains = below.as_ref().is_none_or(|below| *below == base_now);
                        let ancestor = is_ancestor(checkout, &base_now, &head_now).unwrap_or(false);
                        if (moved || below.is_some()) && !(ancestor && chains) {
                            report(layer, DriftKind::Moved);
                        }
                        below = Some(head_now.clone());
                        top = Some(head_now);
                    }
                    Part::Worktree { .. } => {
                        let head = checkout.and_then(|checkout| checkout.head().ok().flatten());
                        let holds = head.as_ref().is_some_and(|head| {
                            *head == kept.head_commit
                                && top.as_ref().is_none_or(|top| top == head)
                        });
                        if !holds {
                            report(layer, DriftKind::HeadMoved);
                        }
                    }
                }
            }
        }
        drift
    }
}
