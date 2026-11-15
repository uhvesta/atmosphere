//! Deterministic reviews for tests, snapshots and timing.
//!
//! [`large`] is the stress fixture: three repos and about 20,000 diff lines,
//! with tabs, wide characters, emoji, right-to-left text and long lines that
//! wrap, every file's whole text known so folds open and full-file mode
//! works. [`small`] is a hand-written review that fits a snapshot, with a
//! viewed file and one changed since it was viewed; [`prepared`] is the
//! same review as an agent prepared it, ordered in groups with notes, one
//! of them outdated. The same seed always gives the same review, on every
//! machine.

use crate::anchor::{LineRange, LineTarget, Side};
use crate::guide::{FileRef, Group, Guide, Note, NoteTarget};
use crate::model::{DiffLine, FileDiff, FileStatus, Hunk, LineKind, RepoDiff, Review, Viewed};
use crate::patch;
use crate::order::{Annotation, AnnotationId};
use crate::spec::{Base, DiffSpec, Include};

/// Diff lines [`large`] holds at least.
pub const LARGE_LINES: usize = 20_000;

const REPOS: [(&str, &str); 3] = [
    ("atmosphere", "rs"),
    ("humata-daemon", "go"),
    ("tca-rust", "ts"),
];

/// Seeded xorshift, so the fixture is the same everywhere.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A number in `range`.
    fn between(&mut self, low: u64, high: u64) -> u64 {
        low + self.next() % (high - low + 1)
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        let at = usize::try_from(self.next() % items.len() as u64).unwrap_or(0);
        items[at]
    }
}

const WORDS: &[&str] = &[
    "balance", "ledger", "account", "session", "machine", "review", "anchor", "cursor", "buffer",
    "render", "snapshot", "journal", "record", "stream", "socket", "channel", "fork", "board",
];

const SPECIAL: &[&str] = &[
    "// 注释：在四舍五入之前先计算余额",
    "let greeting = \"🎉 deployed 👩\u{200d}💻 to 東京\";",
    "// שלום עולם — مرحبا بالعالم",
    "\tif ready {\t// a tab before and after",
    "\t\treturn Err(Error::Closed { at, reason });",
    "let ü = \"naïve café\"; // combining: e\u{301}",
];

fn code_line(random: &mut Random, extension: &str) -> String {
    if random.chance(4) {
        return random.pick(SPECIAL).to_owned();
    }
    let indent = " ".repeat(usize::try_from(random.between(0, 3) * 4).unwrap_or(0));
    let a = random.pick(WORDS);
    let b = random.pick(WORDS);
    let n = random.between(0, 999);
    let body = match (extension, random.between(0, 5)) {
        ("rs", 0) => format!("let {a} = {b}.{a}_{n}()?;"),
        ("rs", 1) => format!("fn {a}_{b}(&self, at: u32) -> Option<{}> {{", capital(b)),
        ("rs", 2) => format!("// {a} keeps its {b} across restarts ({n})"),
        ("go", 0) => format!("{a} := {b}.{}({n})", capital(a)),
        ("go", 1) => format!("func ({a} *{}) {}() error {{", capital(b), capital(a)),
        ("go", 2) => format!("// {a} is written before {b} is read ({n})"),
        ("ts", 0) => format!("const {a} = await {b}.{a}({n});"),
        ("ts", 1) => format!("export function {a}({b}: {}): void {{", capital(b)),
        ("ts", 2) => format!("// {a} follows the {b} it came from ({n})"),
        (_, 3) => format!("if {a} > {n} {{"),
        (_, 4) => "}".to_owned(),
        _ => format!("{a}.push({b}, \"{a}-{n}\");"),
    };
    let mut line = format!("{indent}{body}");
    if random.chance(8) {
        // A long line, 180 to 420 characters, that wraps.
        let target = usize::try_from(random.between(180, 420)).unwrap_or(180);
        while line.len() < target {
            line.push(' ');
            line.push_str(random.pick(WORDS));
            line.push_str(if random.chance(30) { "," } else { "" });
        }
    }
    line
}

fn capital(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

const CONTEXT: usize = 3;

/// A file of `length` lines with changes every so often, hunks consistent
/// with its whole text.
fn file(random: &mut Random, path: String, extension: &str, length: usize) -> FileDiff {
    let new_lines: Vec<String> = (0..length).map(|_| code_line(random, extension)).collect();
    let mut hunks = Vec::new();
    // Old lines minus new lines before the current place.
    let mut shift: i64 = 0;
    let mut at = usize::try_from(random.between(4, 30)).unwrap_or(4);
    while at + CONTEXT * 2 + 12 < length {
        let added = usize::try_from(random.between(0, 8)).unwrap_or(0);
        let removed = usize::try_from(random.between(u64::from(added == 0), 6)).unwrap_or(1);
        let before = at.saturating_sub(CONTEXT);
        let after = (at + added + CONTEXT).min(length);
        let mut lines = Vec::new();
        lines.extend(
            new_lines[before..at]
                .iter()
                .map(|text| DiffLine::new(LineKind::Context, text.clone())),
        );
        lines.extend((0..removed).map(|_| DiffLine::new(LineKind::Removed, code_line(random, extension))));
        lines.extend(
            new_lines[at..at + added]
                .iter()
                .map(|text| DiffLine::new(LineKind::Added, text.clone())),
        );
        lines.extend(
            new_lines[at + added..after]
                .iter()
                .map(|text| DiffLine::new(LineKind::Context, text.clone())),
        );
        let new_start = u32::try_from(before + 1).unwrap_or(u32::MAX);
        let old_start = u32::try_from(i64::from(new_start) + shift).unwrap_or(u32::MAX);
        shift += i64::try_from(removed).unwrap_or(0) - i64::try_from(added).unwrap_or(0);
        hunks.push(Hunk {
            old_start,
            new_start,
            section: format!("{} {}", if extension == "go" { "func" } else { "fn" }, random.pick(WORDS)),
            lines,
        });
        at = after + usize::try_from(random.between(4, 40)).unwrap_or(4) + CONTEXT;
    }
    FileDiff {
        path,
        status: FileStatus::Modified,
        hunks,
        new_lines: Some(new_lines),
        ..FileDiff::default()
    }
}

/// About 20,000 diff lines over three repos, from a fixed seed.
#[must_use]
pub fn large() -> Review {
    let mut random = Random(0x9e37_79b9_7f4a_7c15);
    let mut repos: Vec<RepoDiff> = REPOS
        .iter()
        .map(|(name, _)| RepoDiff {
            name: (*name).to_owned(),
            branch: "humata/review".to_owned(),
            spec: DiffSpec::default(),
            ..RepoDiff::default()
        })
        .collect();
    let mut total = 0;
    let mut count = 0;
    while total < LARGE_LINES {
        let repo = count % REPOS.len();
        let (_, extension) = REPOS[repo];
        let directory = random.pick(&["src", "src/store", "src/view", "tests", "lib"]);
        let path = format!("{directory}/{}_{count}.{extension}", random.pick(WORDS));
        let length = usize::try_from(random.between(120, 900)).unwrap_or(120);
        let diff = file(&mut random, path, extension, length);
        total += diff.hunks.iter().map(|hunk| hunk.lines.len()).sum::<usize>();
        repos[repo].files.push(diff);
        count += 1;
    }
    Review { repos, guide: None }
}

/// Annotations on [`large`]: one on every 40th hunk's first added line (or
/// its first removed line, on the old side), some over several lines.
#[must_use]
pub fn large_annotations(review: &Review) -> Vec<Annotation> {
    let mut notes = Vec::new();
    let mut seen = 0_u64;
    for repo in &review.repos {
        for file in &repo.files {
            for hunk in &file.hunks {
                seen += 1;
                if seen % 40 != 0 {
                    continue;
                }
                let target = changed_line(hunk).map(|(side, number)| LineTarget {
                    repo: repo.name.clone(),
                    path: file.path.clone(),
                    side,
                    lines: if seen % 80 == 0 {
                        LineRange::between(number, number + 1)
                    } else {
                        LineRange::single(number)
                    },
                });
                if let Some(target) = target {
                    notes.push(Annotation {
                        id: AnnotationId(seen),
                        target,
                    });
                }
            }
        }
    }
    notes
}

/// A review as Git patch text, every file one after another and named
/// `repo/path`, as the diff view hands it to gpui-component's diff. With
/// `full_context`, files whose whole text is known hold every line, as
/// with a fold open or full-file mode. For timing the diff on its own.
#[must_use]
pub fn patch_text(review: &Review, full_context: bool) -> String {
    let context = if full_context { patch::Context::Full } else { patch::Context::Hunks };
    review
        .repos
        .iter()
        .flat_map(|repo| repo.files.iter().map(move |file| patch::file_patch(&repo.name, file, context)))
        .collect()
}

/// The first changed line of a hunk: its side and number.
fn changed_line(hunk: &Hunk) -> Option<(Side, u32)> {
    let (mut old, mut new) = (hunk.old_start.max(1), hunk.new_start.max(1));
    for line in &hunk.lines {
        match line.kind {
            LineKind::Context => {
                old += 1;
                new += 1;
            }
            LineKind::Added => return Some((Side::New, new)),
            LineKind::Removed => {
                if hunk.lines.iter().all(|line| line.kind != LineKind::Added) {
                    return Some((Side::Old, old));
                }
                old += 1;
            }
        }
    }
    None
}

/// A small review for snapshots: two repos; a modified Rust file with a fold
/// that opens, a tab, wide characters, an emoji, right-to-left text and a
/// long line; an added file; a renamed file; and a binary file.
#[must_use]
pub fn small() -> Review {
    let ledger: Vec<String> = [
        "use std::fmt;",
        "",
        "/// A balance in cents.",
        "pub struct Cents(i64);",
        "",
        "impl Cents {",
        "    pub fn zero() -> Self {",
        "        Self(0)",
        "    }",
        "",
        "    pub fn add(self, other: Self) -> Self {",
        "        Self(self.0 + other.0)",
        "    }",
        "",
        "    // 注释：在四舍五入之前先计算余额",
        "    pub fn round(self, step: i64) -> Self {",
        "\t\tlet half = step / 2; // tabs",
        "        Self((self.0 + half) / step * step)",
        "    }",
        "}",
        "",
        "pub fn describe(cents: Cents) -> String {",
        "    format!(\"🎉 {} cents, rounded half up so that every account balance reads the same in every report the ledger writes\", cents.0)",
        "}",
        "// שלום עולם — مرحبا بالعالم",
    ]
    .map(str::to_owned)
    .to_vec();
    let context = |at: usize| DiffLine::new(LineKind::Context, ledger[at - 1].clone());
    let added = |at: usize| DiffLine::new(LineKind::Added, ledger[at - 1].clone());
    let ledger_diff = FileDiff {
        path: "src/ledger.rs".to_owned(),
        status: FileStatus::Modified,
        hunks: vec![
            Hunk {
                old_start: 1,
                new_start: 1,
                section: String::new(),
                lines: vec![
                    context(1),
                    context(2),
                    DiffLine::new(LineKind::Removed, "/// Cents."),
                    added(3),
                    context(4),
                    context(5),
                    context(6),
                ],
            },
            Hunk {
                old_start: 12,
                new_start: 12,
                section: "impl Cents".to_owned(),
                lines: vec![
                    context(12),
                    context(13),
                    context(14),
                    added(15),
                    DiffLine::new(LineKind::Removed, "    pub fn round(self) -> Self {"),
                    DiffLine::new(LineKind::Removed, "        self"),
                    added(16),
                    added(17),
                    added(18),
                    context(19),
                    context(20),
                    context(21),
                    context(22),
                    DiffLine::new(LineKind::Removed, "    format!(\"{} cents\", cents.0)"),
                    added(23),
                    context(24),
                    added(25),
                ],
            },
        ],
        new_lines: Some(ledger),
        content_id: Some("blob:ledger-2".to_owned()),
        ..FileDiff::default()
    };
    let readme = FileDiff {
        path: "docs/ROUNDING.md".to_owned(),
        status: FileStatus::Added,
        hunks: vec![Hunk {
            old_start: 0,
            new_start: 1,
            section: String::new(),
            lines: vec![
                DiffLine::new(LineKind::Added, "# Rounding"),
                DiffLine::new(LineKind::Added, ""),
                DiffLine::new(LineKind::Added, "Balances round half up, to the step the account sets."),
            ],
        }],
        new_lines: None,
        viewed: Viewed::Yes,
        content_id: Some("blob:rounding-1".to_owned()),
    };
    let renamed = FileDiff {
        path: "src/client.go".to_owned(),
        status: FileStatus::Renamed {
            from: "src/old_client.go".to_owned(),
        },
        hunks: vec![Hunk {
            old_start: 8,
            new_start: 8,
            section: "func Dial".to_owned(),
            lines: vec![
                DiffLine::new(LineKind::Context, "func Dial(address string) (*Client, error) {"),
                DiffLine::new(LineKind::Removed, "\tconn, err := net.Dial(\"unix\", address)"),
                DiffLine::new(LineKind::Added, "\tconn, err := net.DialTimeout(\"unix\", address, timeout)"),
                DiffLine::new(LineKind::Context, "\tif err != nil {"),
            ],
        }],
        new_lines: None,
        viewed: Viewed::ChangedSince,
        content_id: Some("blob:client-3".to_owned()),
    };
    let binary = FileDiff {
        path: "assets/icon.png".to_owned(),
        status: FileStatus::Modified,
        hunks: Vec::new(),
        ..FileDiff::default()
    };
    Review {
        repos: vec![
            RepoDiff {
                name: "atmosphere".to_owned(),
                branch: "humata/rounding".to_owned(),
                spec: DiffSpec::default(),
                base_commit: Some("4f2a9c1".to_owned()),
                bases: vec![
                    Base::Ref("main".to_owned()),
                    Base::Reviewed {
                        number: 1,
                        label: "review 1, Tuesday 14:02".to_owned(),
                    },
                ],
                files: vec![ledger_diff, readme],
            },
            RepoDiff {
                name: "humata-daemon".to_owned(),
                branch: "humata/dial-timeout".to_owned(),
                spec: DiffSpec {
                    include: Include::Staged,
                    ..DiffSpec::default()
                },
                base_commit: None,
                bases: Vec::new(),
                files: vec![renamed, binary],
            },
        ],
        guide: None,
    }
}

/// [`small`] as an agent prepared it (RV-32): the daemon's change first,
/// then the ledger's, with notes on a file, a hunk and lines; the note on
/// line 23 quotes code that has changed since, so it is outdated. The
/// binary file isn't named, so it comes last under "Other changes".
#[must_use]
pub fn prepared() -> Review {
    let mut review = small();
    let ledger = |side: Side, start: u32, end: u32| LineTarget {
        repo: "atmosphere".to_owned(),
        path: "src/ledger.rs".to_owned(),
        side,
        lines: LineRange::between(start, end),
    };
    review.guide = Some(Guide {
        author: "agent · Haiku".to_owned(),
        groups: vec![
            Group {
                title: "Entry point: dialing with a timeout".to_owned(),
                files: vec![FileRef::new("humata-daemon", "src/client.go")],
            },
            Group {
                title: "Rounding, leaf first".to_owned(),
                files: vec![
                    FileRef::new("atmosphere", "src/ledger.rs"),
                    FileRef::new("atmosphere", "src/gone.rs"),
                    FileRef::new("atmosphere", "docs/ROUNDING.md"),
                    FileRef::new("atmosphere", "src/ledger.rs"),
                ],
            },
        ],
        notes: vec![
            Note {
                target: NoteTarget::File(FileRef::new("atmosphere", "src/ledger.rs")),
                text: "Start here: round() now takes the step it rounds to.".to_owned(),
            },
            Note {
                target: NoteTarget::Hunk {
                    file: FileRef::new("atmosphere", "src/ledger.rs"),
                    side: Side::New,
                    line: 16,
                },
                text: "The signature change; every caller passes the account's step.".to_owned(),
            },
            Note {
                target: NoteTarget::Lines {
                    target: ledger(Side::New, 17, 18),
                    quoted: "\t\tlet half = step / 2; // tabs\n        Self((self.0 + half) / step * step)"
                        .to_owned(),
                },
                text: "Half up: adding half the step before dividing.".to_owned(),
            },
            Note {
                target: NoteTarget::Lines {
                    target: ledger(Side::New, 23, 23),
                    quoted: "    format!(\"{} cents rounded\", cents.0)".to_owned(),
                },
                text: "Say which rounding was used.".to_owned(),
            },
        ],
    });
    review
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::line_text;

    #[test]
    fn the_large_review_is_the_same_every_time_and_big_enough() {
        let review = large();
        assert_eq!(review, large());
        assert!(review.line_count() >= LARGE_LINES);
        assert_eq!(review.repos.len(), 3);
        assert!(review.repos.iter().all(|repo| !repo.files.is_empty()));
    }

    #[test]
    fn the_large_reviews_hunks_agree_with_the_whole_files() {
        for file in large().repos.iter().flat_map(|repo| &repo.files) {
            let lines = file.new_lines.as_ref().expect("the fixture knows every file");
            for hunk in &file.hunks {
                let mut new = hunk.new_start as usize;
                for line in &hunk.lines {
                    if line.kind != LineKind::Removed {
                        assert_eq!(lines[new - 1], line.text, "{} line {new}", file.path);
                        new += 1;
                    }
                }
            }
            for pair in file.hunks.windows(2) {
                assert!(pair[0].new_end() < pair[1].new_first(), "hunks overlap in {}", file.path);
                assert!(pair[0].old_end() < pair[1].old_first(), "hunks overlap in {}", file.path);
            }
        }
    }

    #[test]
    fn the_large_review_has_long_and_unusual_lines() {
        let review = large();
        let texts: Vec<&str> = review
            .repos
            .iter()
            .flat_map(|repo| &repo.files)
            .flat_map(|file| file.new_lines.iter().flatten())
            .map(String::as_str)
            .collect();
        assert!(texts.iter().any(|text| text.len() > 300));
        assert!(texts.iter().any(|text| text.contains('\t')));
        assert!(texts.iter().any(|text| text.contains('注')));
        assert!(texts.iter().any(|text| text.contains('🎉')));
        assert!(texts.iter().any(|text| text.contains('ש')));
    }

    #[test]
    fn annotations_point_at_lines_the_review_holds() {
        let review = large();
        let notes = large_annotations(&review);
        assert!(notes.len() > 20);
        for note in &notes {
            let file = review
                .repos
                .iter()
                .find(|repo| repo.name == note.target.repo)
                .and_then(|repo| repo.files.iter().find(|file| file.path == note.target.path))
                .expect("the note's file is in the review");
            assert!(line_text(file, note.target.side, note.target.lines.start).is_some());
        }
    }

    #[test]
    fn the_small_reviews_hunks_agree_with_its_whole_file() {
        let review = small();
        let file = &review.repos[0].files[0];
        let lines = file.new_lines.as_ref().expect("the ledger's text is known");
        for hunk in &file.hunks {
            let mut new = hunk.new_start as usize;
            for line in &hunk.lines {
                if line.kind != LineKind::Removed {
                    assert_eq!(lines[new - 1], line.text);
                    new += 1;
                }
            }
        }
    }
}
