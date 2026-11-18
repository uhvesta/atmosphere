//! The reading order of a review's files, bottom-up (§26.17 step 4, RV-33):
//! the leaf changes first, so a file is read after the files whose code it
//! uses. A file uses another when its code names that file's module (its
//! file stem) as a whole word; modules that name each other keep the order
//! they came in.

use std::collections::HashSet;
use std::path::Path;

/// One changed file, as far as ordering needs it.
pub struct OrderFile<'a> {
    /// The file, relative to its repository's root.
    pub path: &'a str,
    /// The file's code as the diff shows it.
    pub text: &'a str,
}

/// Names too common to say a file is used (entry files and indexes).
const COMMON: [&str; 6] = ["mod", "lib", "main", "index", "init", "test"];

fn stem(path: &str) -> Option<&str> {
    let stem = Path::new(path).file_stem()?.to_str()?;
    (stem.len() >= 3 && !COMMON.contains(&stem)).then_some(stem)
}

fn words(text: &str) -> HashSet<&str> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
        .collect()
}

/// The indexes of `files` in bottom-up reading order: a file comes after
/// every file it uses. Files that use nothing, or only each other, keep their
/// given order among themselves.
#[must_use]
pub fn bottom_up(files: &[OrderFile<'_>]) -> Vec<usize> {
    let words: Vec<HashSet<&str>> = files.iter().map(|file| words(file.text)).collect();
    let uses = |user: usize, used: usize| {
        user != used && stem(files[used].path).is_some_and(|stem| words[user].contains(stem))
    };
    let mut placed = vec![false; files.len()];
    let mut order = Vec::with_capacity(files.len());
    while order.len() < files.len() {
        let ready = (0..files.len()).find(|&file| {
            !placed[file] && (0..files.len()).all(|used| placed[used] || !uses(file, used))
        });
        // A cycle: the earliest file left goes next.
        let next = ready.or_else(|| placed.iter().position(|done| !done));
        let Some(next) = next else { break };
        placed[next] = true;
        order.push(next);
    }
    order
}
