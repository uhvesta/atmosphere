//! Diffing two trees: the file list with counts and content hashes in one
//! `git diff-tree`, then each file's hunks on demand, streamed from git and
//! cut into chunks so a huge file never sits in memory whole.

use std::io::{BufRead, BufReader};
use std::process::Stdio;

use crate::checkout::Checkout;
use crate::error::DiffError;
use crate::git::{Git, trimmed};
use crate::model::{
    Budget, Cursor, DiffLine, FileChunk, FileStatus, FileSummary, Hunk, LineKind, RepoDiff, Trees,
    Viewed,
};
use crate::spec::{Base, BaseContext, RepoSpec, Untracked, Upto, Whitespace};

/// How lines are matched, from the spec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Matching {
    pub whitespace: Whitespace,
    pub renames: bool,
}

impl From<&RepoSpec> for Matching {
    fn from(spec: &RepoSpec) -> Self {
        Self {
            whitespace: spec.whitespace,
            renames: spec.renames,
        }
    }
}

impl Matching {
    fn args(self) -> Vec<&'static str> {
        let mut args = vec![if self.renames { "-M" } else { "--no-renames" }];
        match self.whitespace {
            Whitespace::Counts => {}
            Whitespace::IgnoreChanges => args.push("-b"),
            Whitespace::IgnoreAll => args.push("-w"),
        }
        args
    }
}

impl Checkout {
    /// The files `spec` shows now, with counts and content hashes; no
    /// hunks (ask [`Self::file_chunk_blocking`] for those). Blocking: call
    /// [`Self::diff`] from async code.
    ///
    /// # Errors
    ///
    /// [`DiffError`] when a base doesn't resolve or git fails.
    pub fn diff_blocking(
        &self,
        spec: &RepoSpec,
        context: &BaseContext,
    ) -> Result<RepoDiff, DiffError> {
        let trees = self.trees(spec, context)?;
        let files = self.files(&trees, spec)?;
        self.summary(spec, context, trees, files)
    }

    /// The files between two trees under `spec`'s paths and matching.
    pub(crate) fn files(&self, trees: &Trees, spec: &RepoSpec) -> Result<Vec<FileSummary>, DiffError> {
        let mut args: Vec<String> = ["diff-tree", "-r", "-z", "--raw", "--numstat", "--no-ext-diff"]
            .map(str::to_owned)
            .to_vec();
        args.extend(Matching::from(spec).args().into_iter().map(str::to_owned));
        args.push(trees.old.clone());
        args.push(trees.new.clone());
        let specs = spec.paths.pathspecs();
        if !specs.is_empty() {
            args.push("--".to_owned());
            args.extend(specs);
        }
        let output = self.git.run(&args)?;
        parse_listing(&output).ok_or_else(|| DiffError::BadOutput {
            repo: self.path().to_owned(),
            what: "the raw and numstat listing of git diff-tree".to_owned(),
        })
    }

    /// One file's hunks between `trees`, from `from` until `budget` runs
    /// out. Only the file's own paths are diffed, and git's output is read
    /// as it streams and stopped once the chunk is full.
    ///
    /// # Errors
    ///
    /// [`DiffError`] when git fails.
    pub fn file_chunk_blocking(
        &self,
        trees: &Trees,
        file: &FileSummary,
        matching: Matching,
        from: Cursor,
        budget: Budget,
    ) -> Result<FileChunk, DiffError> {
        let mut args: Vec<String> = vec![
            "--literal-pathspecs".to_owned(),
            "diff-tree".to_owned(),
            "-p".to_owned(),
            "--no-color".to_owned(),
            "--no-ext-diff".to_owned(),
        ];
        args.extend(matching.args().into_iter().map(str::to_owned));
        args.extend([trees.old.clone(), trees.new.clone(), "--".to_owned()]);
        args.push(file.old_path().to_owned());
        if file.old_path() != file.path {
            args.push(file.path.clone());
        }
        stream_chunk(&self.git, &args, from, budget)
    }

    /// What a file marked Viewed at `viewed_blob` shows now (RV-29): only
    /// the change from that content to the file's current content.
    ///
    /// # Errors
    ///
    /// [`DiffError`] when git fails.
    pub fn since_viewed_blocking(
        &self,
        viewed_blob: &str,
        now: &FileSummary,
        matching: Matching,
        from: Cursor,
        budget: Budget,
    ) -> Result<Viewed, DiffError> {
        if now.new_blob.as_deref() == Some(viewed_blob) {
            return Ok(Viewed::Unchanged);
        }
        let exists = self
            .git
            .output(["cat-file", "-e", &format!("{viewed_blob}^{{blob}}")], None)?;
        if !exists.status.success() {
            return Ok(Viewed::Forgotten);
        }
        let current = match &now.new_blob {
            Some(blob) => blob.clone(),
            None => trimmed(
                &self
                    .git
                    .run_with_input(["hash-object", "-w", "--stdin"], b"")?,
            ),
        };
        let mut args: Vec<String> = ["diff", "--no-color", "--no-ext-diff"]
            .map(str::to_owned)
            .to_vec();
        args.extend(
            matching
                .args()
                .into_iter()
                .filter(|arg| !matches!(*arg, "-M" | "--no-renames"))
                .map(str::to_owned),
        );
        args.extend([viewed_blob.to_owned(), current]);
        Ok(Viewed::Changed(stream_chunk(&self.git, &args, from, budget)?))
    }

    /// The lines of a blob, for anchoring notes.
    pub(crate) fn blob_lines(&self, blob: &str) -> Result<Vec<String>, DiffError> {
        let bytes = self.git.run(["cat-file", "blob", blob])?;
        Ok(String::from_utf8_lossy(&bytes)
            .lines()
            .map(str::to_owned)
            .collect())
    }
}

/// RV-31's "Comparing: …" line for a spec.
pub(crate) fn label(spec: &RepoSpec) -> String {
    let mut label = match &spec.base {
        Base::MergeBase { with } => {
            format!("{with}...{}", spec.compare.head.as_deref().unwrap_or("HEAD"))
        }
        Base::Rev { rev } if rev == "HEAD" => "HEAD".to_owned(),
        Base::Rev { rev } => format!("{rev}..{}", spec.compare.head.as_deref().unwrap_or("HEAD")),
        Base::Snapshot { n: Some(n) } => format!("since review {n}"),
        Base::Snapshot { n: None } => "since last review".to_owned(),
        Base::ForkPoint => "since the fork".to_owned(),
        Base::Index => "staged".to_owned(),
    };
    match (spec.compare.upto, &spec.base) {
        (Upto::Commits, _) => {}
        (Upto::Staged, Base::Index) => {}
        (Upto::Staged, _) => label.push_str(" + staged"),
        (Upto::Unstaged, Base::Index) => label = "unstaged".to_owned(),
        (Upto::Unstaged, _) => label.push_str(" + staged + unstaged"),
    }
    match &spec.compare.untracked {
        Untracked::None => {}
        Untracked::All => label.push_str(" + untracked"),
        Untracked::Paths(paths) => {
            label.push_str(" + untracked in ");
            label.push_str(&paths.join(", "));
        }
    }
    if !spec.compare.worktree_paths.is_empty() {
        label.push_str(" + ");
        label.push_str(&spec.compare.worktree_paths.join(", "));
    }
    if !spec.paths.include.is_empty() {
        label.push_str(" (only ");
        label.push_str(&spec.paths.include.join(", "));
        label.push(')');
    }
    label
}

/// Reads `diff-tree -r -z --raw --numstat`: every raw entry, then every
/// numstat entry in the same order.
fn parse_listing(output: &[u8]) -> Option<Vec<FileSummary>> {
    let mut tokens = output
        .split(|byte| *byte == 0)
        .map(|token| String::from_utf8_lossy(token).into_owned())
        .peekable();
    let mut files = Vec::new();
    while tokens.peek().is_some_and(|token| token.starts_with(':')) {
        let meta = tokens.next()?;
        let fields: Vec<&str> = meta.trim_start_matches(':').split(' ').collect();
        let [old_mode, new_mode, old, new, status] = fields.as_slice() else {
            return None;
        };
        let blob = |oid: &str| (!oid.bytes().all(|byte| byte == b'0')).then(|| oid.to_owned());
        let letter = status.chars().next()?;
        let (path, status) = match letter {
            'R' => {
                let from = tokens.next()?;
                let to = tokens.next()?;
                let similarity = status.get(1..).and_then(|score| score.parse().ok()).unwrap_or(0);
                (to, FileStatus::Renamed { from, similarity })
            }
            'C' => {
                let _from = tokens.next()?;
                (tokens.next()?, FileStatus::Added)
            }
            'A' => (tokens.next()?, FileStatus::Added),
            'D' => (tokens.next()?, FileStatus::Deleted),
            _ => (tokens.next()?, FileStatus::Modified),
        };
        files.push(FileSummary {
            path,
            status,
            added: 0,
            removed: 0,
            binary: false,
            old_blob: blob(old),
            new_blob: blob(new),
            old_mode: old_mode.parse().unwrap_or(0),
            new_mode: new_mode.parse().unwrap_or(0),
        });
    }
    for file in &mut files {
        let entry = tokens.next()?;
        let mut parts = entry.splitn(3, '\t');
        let added = parts.next()?;
        let removed = parts.next()?;
        if parts.next()?.is_empty() {
            // A rename: its two paths follow as their own tokens.
            tokens.next()?;
            tokens.next()?;
        }
        if added == "-" {
            file.binary = true;
        } else {
            file.added = added.parse().ok()?;
            file.removed = removed.parse().ok()?;
        }
    }
    Some(files)
}

/// Runs a diff of one file and keeps the lines from `from` until the
/// budget runs out, then stops git.
fn stream_chunk(git: &Git, args: &[String], from: Cursor, budget: Budget) -> Result<FileChunk, DiffError> {
    let mut command = git.command(args);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = git.spawn(&mut command)?;
    let Some(stdout) = child.stdout.take() else {
        return Err(DiffError::BadOutput {
            repo: git.workdir().to_owned(),
            what: "no output pipe from git diff".to_owned(),
        });
    };
    let mut reader = BufReader::with_capacity(64 * 1024, stdout);
    let mut parser = ChunkParser::new(from, budget);
    let mut line = Vec::new();
    let mut stopped = false;
    loop {
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .map_err(|source| DiffError::io("read git's diff", git.workdir(), source))?;
        if read == 0 {
            break;
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if parser.feed(&line) == Feed::Full {
            stopped = true;
            break;
        }
    }
    drop(reader);
    if stopped {
        // The rest isn't needed; git may be blocked writing it.
        let _ = child.kill();
        let _ = child.wait();
        return Ok(parser.finish());
    }
    let output = child
        .wait_with_output()
        .map_err(|source| DiffError::io("wait for git diff", git.workdir(), source))?;
    if !output.status.success() {
        return Err(git.failed(args, &output));
    }
    Ok(parser.finish())
}

#[derive(Debug, PartialEq, Eq)]
enum Feed {
    More,
    Full,
}

/// Turns one file's unified diff into a chunk, line by line.
struct ChunkParser {
    from: Cursor,
    budget: Budget,
    chunk: FileChunk,
    /// The current hunk's index, once one started.
    hunk: Option<u32>,
    line: u32,
    old: u32,
    new: u32,
    lines: usize,
    bytes: usize,
    /// The previous diff line is the last one in the chunk.
    kept_last: bool,
}

impl ChunkParser {
    fn new(from: Cursor, budget: Budget) -> Self {
        Self {
            from,
            budget,
            chunk: FileChunk::default(),
            hunk: None,
            line: 0,
            old: 0,
            new: 0,
            lines: 0,
            bytes: 0,
            kept_last: false,
        }
    }

    fn full(&self) -> bool {
        self.lines > 0 && (self.lines >= self.budget.max_lines || self.bytes >= self.budget.max_bytes)
    }

    fn feed(&mut self, raw: &[u8]) -> Feed {
        if raw.starts_with(b"@@") {
            let index = self.hunk.map_or(0, |hunk| hunk + 1);
            let Some((old_start, new_start, section)) = parse_header(raw) else {
                return Feed::More;
            };
            self.hunk = Some(index);
            self.line = 0;
            self.old = old_start.max(1);
            self.new = new_start.max(1);
            let here = Cursor { hunk: index, line: 0 };
            if here >= self.from && self.full() {
                self.chunk.next = Some(here);
                return Feed::Full;
            }
            if index >= self.from.hunk {
                self.chunk.hunks.push(Hunk {
                    index,
                    old_start,
                    new_start,
                    section,
                    continued: index == self.from.hunk && self.from.line > 0,
                    lines: Vec::new(),
                });
            }
            return Feed::More;
        }
        let Some(hunk) = self.hunk else {
            if raw.starts_with(b"Binary files ") || raw.starts_with(b"GIT binary patch") {
                self.chunk.binary = true;
            }
            return Feed::More;
        };
        let (kind, text) = match raw.first() {
            Some(b'+') => (LineKind::Added, &raw[1..]),
            Some(b'-') => (LineKind::Removed, &raw[1..]),
            Some(b' ') => (LineKind::Context, &raw[1..]),
            Some(b'\\') => {
                if self.kept_last
                    && let Some(last) = self.chunk.hunks.last_mut().and_then(|hunk| hunk.lines.last_mut())
                {
                    last.no_newline = true;
                }
                return Feed::More;
            }
            Some(_) => (LineKind::Context, raw),
            None => (LineKind::Context, raw),
        };
        self.kept_last = false;
        let (old_line, new_line) = match kind {
            LineKind::Context => (Some(self.old), Some(self.new)),
            LineKind::Added => (None, Some(self.new)),
            LineKind::Removed => (Some(self.old), None),
        };
        if old_line.is_some() {
            self.old += 1;
        }
        if new_line.is_some() {
            self.new += 1;
        }
        let here = Cursor {
            hunk,
            line: self.line,
        };
        self.line += 1;
        if here < self.from {
            return Feed::More;
        }
        if self.full() {
            self.chunk.next = Some(here);
            return Feed::Full;
        }
        if let Some(current) = self.chunk.hunks.last_mut() {
            current.lines.push(DiffLine {
                kind,
                old_line,
                new_line,
                text: String::from_utf8_lossy(text).into_owned(),
                no_newline: false,
            });
            self.kept_last = true;
            self.lines += 1;
            self.bytes += text.len() + 1;
        }
        Feed::More
    }

    fn finish(self) -> FileChunk {
        self.chunk
    }
}

/// `@@ -a,b +c,d @@ section`: the old and new starts and the section.
fn parse_header(raw: &[u8]) -> Option<(u32, u32, String)> {
    let text = String::from_utf8_lossy(raw);
    let rest = text.strip_prefix("@@ ")?;
    let (ranges, section) = rest.split_once(" @@")?;
    let (old, new) = ranges.split_once(' ')?;
    let start = |range: &str| -> Option<u32> {
        range.split(',').next()?.parse().ok()
    };
    Some((
        start(old.strip_prefix('-')?)?,
        start(new.strip_prefix('+')?)?,
        section.trim_start().to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(diff: &str, from: Cursor, max_lines: usize) -> FileChunk {
        let mut parser = ChunkParser::new(
            from,
            Budget {
                max_lines,
                max_bytes: usize::MAX,
            },
        );
        for line in diff.lines() {
            if parser.feed(line.as_bytes()) == Feed::Full {
                break;
            }
        }
        parser.finish()
    }

    const DIFF: &str = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@ fn a\n-one\n+uno\n two\n@@ -10,1 +10,2 @@\n ten\n+eleven\n";

    #[test]
    fn line_numbers_follow_each_side() {
        let chunk = chunk(DIFF, Cursor::default(), 100);
        assert_eq!(chunk.hunks.len(), 2);
        let first = &chunk.hunks[0];
        assert_eq!(first.section, "fn a");
        assert_eq!(first.lines[0].old_line, Some(1));
        assert_eq!(first.lines[0].new_line, None);
        assert_eq!(first.lines[1].new_line, Some(1));
        assert_eq!(first.lines[2].old_line, Some(2));
        assert_eq!(chunk.hunks[1].lines[1].new_line, Some(11));
        assert_eq!(chunk.next, None);
    }

    #[test]
    fn chunks_resume_where_the_last_one_stopped() {
        let first = chunk(DIFF, Cursor::default(), 2);
        assert_eq!(first.hunks[0].lines.len(), 2);
        let next = first.next.expect("more follows");
        let second = chunk(DIFF, next, 2);
        assert!(second.hunks[0].continued);
        assert_eq!(second.hunks[0].lines[0].text, "two");
        assert_eq!(second.hunks.len(), 2);
        assert_eq!(second.hunks[1].lines[0].text, "ten");
        let third = chunk(DIFF, second.next.expect("more follows"), 100);
        assert_eq!(third.hunks.len(), 1);
        assert!(third.hunks[0].continued);
        assert_eq!(third.hunks[0].lines[0].text, "eleven");
        assert_eq!(third.hunks[0].lines[0].new_line, Some(11));
        assert_eq!(third.next, None);
    }

    #[test]
    fn a_rename_listing_reads_both_paths_and_counts() {
        let raw = b":100644 100644 aaaa bbbb R090\0old.rs\0new.rs\0:000000 100644 0000 cccc A\0add.bin\x003\t1\t\0old.rs\0new.rs\0-\t-\tadd.bin\0";
        let files = parse_listing(raw).expect("well formed");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "new.rs");
        assert_eq!(files[0].old_path(), "old.rs");
        assert_eq!((files[0].added, files[0].removed), (3, 1));
        assert!(files[1].binary);
        assert_eq!(files[1].old_blob, None);
    }
}
