//! A stack review's layer list (D800, DS-60 to DS-65): which row is picked,
//! stepping through the rows, and the rows as the Diff panel draws them.
//! Row 0 is Whole stack; rows 1..=N are the layers, 1 at the bottom.

/// The layer list as the Diff panel draws it, top-down: Whole stack, then
/// the layers from the top of the stack (N) down to 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackRows {
    /// The rows in drawing order.
    pub rows: Vec<StackRow>,
    /// Which row is picked, as an index into `rows`.
    pub picked: usize,
    /// The header's comparing line: which layer of how many.
    pub header: StackHeader,
    /// The repositories the picked row shows, as `owner/name`.
    pub shown_repos: Vec<String>,
    /// The session's repositories the picked layer doesn't touch (DS-63).
    pub untouched_repos: Vec<String>,
}

/// One row of the layer list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StackRow {
    /// The layer's number, 1 at the bottom; `None` for Whole stack.
    pub number: Option<usize>,
    /// The layer's title; empty for Whole stack.
    pub title: String,
    /// The one-line purpose, shown only on the picked row; empty for none.
    pub purpose: String,
    /// The pull requests, short: `api #412`, in the layer's repo order.
    pub pull_requests: Vec<String>,
    /// How many files the row's diff changes.
    pub files: u32,
    /// The repositories of this layer that moved since it was sent (DS-65).
    pub drifted: Vec<String>,
    /// The repositories the row touches, as `owner/name`, in the layer's
    /// order; Whole stack lists every layer's, once each.
    pub repos: Vec<String>,
}

/// What the header says the panel compares (DS-62).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StackHeader {
    /// Whole stack, over this many layers.
    Whole {
        /// How many layers the stack has.
        layers: usize,
    },
    /// Layer `number` of `of`, with its title.
    Layer {
        /// 1 at the bottom.
        number: usize,
        /// How many layers the stack has.
        of: usize,
        /// The layer's title.
        title: String,
    },
}

use humata2_clients::model::DiffBase;
use humata2_clients::views::{LayerRepoView, LayerView, StackView};

/// The pull request as the layer list says it: `api #412` for
/// `owner/api#412`. `None` for no pull request.
#[must_use]
pub fn pull_request_label(pull_request: &str) -> Option<String> {
    let pull_request = pull_request.trim();
    if pull_request.is_empty() {
        return None;
    }
    let (repo, number) = match pull_request.rsplit_once('#') {
        Some((repo, number)) => (repo, Some(number)),
        None => (pull_request, None),
    };
    let name = repo.rsplit('/').next().unwrap_or(repo);
    Some(match number {
        Some(number) if !name.is_empty() => format!("{name} #{number}"),
        Some(number) => format!("#{number}"),
        None => name.to_owned(),
    })
}

/// The layer called `title`, by its exact title.
fn layer_named<'a>(stack: &'a StackView, title: &str) -> Option<&'a LayerView> {
    stack.layers.iter().find(|layer| layer.title == title)
}

/// The picked layer's index in the stack (0 at the bottom), or `None` for
/// Whole stack, including a title the stack no longer has.
fn index_of(stack: &StackView, picked: Option<&str>) -> Option<usize> {
    let title = picked?;
    stack.layers.iter().position(|layer| layer.title == title)
}

/// The diff to read for the picked layer: its repositories' specs, or the
/// stack's `whole` for Whole stack (and for a title that no longer exists).
#[must_use]
pub fn layer_base(stack: &StackView, picked: Option<&str>) -> DiffBase {
    match picked.and_then(|title| layer_named(stack, title)) {
        Some(layer) => DiffBase::Spec(layer.repos.iter().map(|repo| repo.spec.clone()).collect()),
        None => DiffBase::Spec(stack.whole.clone()),
    }
}

/// Which way to step through the layers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Toward the top of the stack.
    Next,
    /// Back toward Whole stack.
    Previous,
}

/// The layer after stepping from `picked`: Whole stack, then layer 1 (the
/// bottom) up to N. No wrap: stepping past either end stays where it is.
#[must_use]
pub fn step(stack: &StackView, picked: Option<&str>, way: Step) -> Option<String> {
    let at = index_of(stack, picked);
    let to = match (way, at) {
        (Step::Next, None) => 0,
        (Step::Next, Some(index)) => (index + 1).min(stack.layers.len().saturating_sub(1)),
        (Step::Previous, None | Some(0)) => return None,
        (Step::Previous, Some(index)) => index - 1,
    };
    stack.layers.get(to).map(|layer| layer.title.clone())
}

fn pull_requests(repos: &[LayerRepoView], into: &mut Vec<String>) {
    for repo in repos {
        if let Some(label) = pull_request_label(&repo.pull_request)
            && !into.contains(&label)
        {
            into.push(label);
        }
    }
}

/// The layer list for the Diff panel. `session_repos` are the session's
/// repositories (`owner/name`); `whole_files` is how many files Whole stack
/// changes when it has been read, else the layers' counts are added.
#[must_use]
pub fn rows(
    stack: &StackView,
    picked: Option<&str>,
    session_repos: &[String],
    whole_files: Option<u32>,
) -> StackRows {
    let at = index_of(stack, picked);
    let of = stack.layers.len();
    let mut whole = StackRow::default();
    for layer in &stack.layers {
        pull_requests(&layer.repos, &mut whole.pull_requests);
        whole.files += layer.repos.iter().map(|repo| repo.files).sum::<u32>();
        for repo in &layer.repos {
            if !whole.repos.contains(&repo.repo) {
                whole.repos.push(repo.repo.clone());
            }
        }
        for repo in layer.repos.iter().filter(|repo| repo.drift.is_some()) {
            if !whole.drifted.contains(&repo.repo) {
                whole.drifted.push(repo.repo.clone());
            }
        }
    }
    if let Some(files) = whole_files {
        whole.files = files;
    }
    let mut rows = vec![whole];
    for (index, layer) in stack.layers.iter().enumerate().rev() {
        let mut row = StackRow {
            number: Some(index + 1),
            title: layer.title.clone(),
            purpose: if at == Some(index) {
                layer.purpose.clone()
            } else {
                String::new()
            },
            files: layer.repos.iter().map(|repo| repo.files).sum(),
            repos: layer.repos.iter().map(|repo| repo.repo.clone()).collect(),
            ..StackRow::default()
        };
        pull_requests(&layer.repos, &mut row.pull_requests);
        row.drifted = layer
            .repos
            .iter()
            .filter(|repo| repo.drift.is_some())
            .map(|repo| repo.repo.clone())
            .collect();
        rows.push(row);
    }
    let (picked_row, header, shown_repos, untouched_repos) = match at {
        None => (
            0,
            StackHeader::Whole { layers: of },
            stack.whole.iter().map(|spec| spec.repo.clone()).collect(),
            Vec::new(),
        ),
        Some(index) => {
            let layer = &stack.layers[index];
            let shown: Vec<String> = layer.repos.iter().map(|repo| repo.repo.clone()).collect();
            let untouched = session_repos
                .iter()
                .filter(|repo| !shown.contains(repo))
                .cloned()
                .collect();
            (
                of - index,
                StackHeader::Layer {
                    number: index + 1,
                    of,
                    title: layer.title.clone(),
                },
                shown,
                untouched,
            )
        }
    };
    StackRows {
        rows,
        picked: picked_row,
        header,
        shown_repos,
        untouched_repos,
    }
}

/// How much a file matters to the reader (D802).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tier {
    /// Read this first: it carries a dot.
    Key,
    /// Read it in order.
    #[default]
    Normal,
    /// Safe to skim: it folds into one row at the end of its repository.
    Skim,
}

/// One repository's files split for drawing (DS-67): the files to draw in
/// their order, then the skim files that fold into one row at the end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folded<'a, F> {
    /// The key and normal files, in their order.
    pub shown: Vec<&'a F>,
    /// The skim files, in their order.
    pub skims: Vec<&'a F>,
}

/// Splits `files` by `tier`, keeping each half in its order.
pub fn fold_skims<'a, F>(files: &'a [F], tier: impl Fn(&F) -> Tier) -> Folded<'a, F> {
    let (skims, shown) = files.iter().partition(|file| tier(file) == Tier::Skim);
    Folded { shown, skims }
}

/// The most file names the skim row lists before it says "…".
const SKIM_NAMES: usize = 3;

/// The skim row's words: "4 files to skim · Cargo.lock, schema.rs, mod.rs, …",
/// with each path's file name. Empty for no paths.
#[must_use]
pub fn skim_label(paths: &[&str]) -> String {
    if paths.is_empty() {
        return String::new();
    }
    let count = paths.len();
    let mut names = paths
        .iter()
        .take(SKIM_NAMES)
        .map(|path| path.rsplit('/').next().unwrap_or(path))
        .collect::<Vec<_>>()
        .join(", ");
    if count > SKIM_NAMES {
        names.push_str(", \u{2026}");
    }
    let files = if count == 1 { "file" } else { "files" };
    format!("{count} {files} to skim \u{b7} {names}")
}

/// What the agent is told when a repository of a layer moved after it sent
/// the stack (DS-65): which layer and repository, and what to do.
#[must_use]
pub fn ask_again_text(layer: &str, repo: &str) -> String {
    format!(
        "{repo} moved after you sent the stack for review, so layer \"{layer}\" no \
         longer matches it. Read the stack again and send the review again \
         (review_prepare with from_stack or stack) so each layer points at what \
         {repo} has now."
    )
}
