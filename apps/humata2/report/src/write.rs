//! Saving a scrubbed report as `reports/<date>-<slug>/`.
//!
//! The folder is written whole under a temporary name and renamed into
//! place, so a crash leaves either the finished folder or a partial one
//! whose name ends in `.partial`, never a report missing files. The folder
//! is readable only by this user (0700), and so is each file (0600).

use std::fmt::Write as _;
use std::fs::{self, DirBuilder, File, OpenOptions, Permissions};
use std::io::{ErrorKind, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::change;
use crate::error::{FormError, WriteError};
use crate::event::{Event, Gesture, Holds, Kind};
use crate::form::{Drawing, Form};
use crate::jsonl;
use crate::picture;
use crate::recording::Captured;
use crate::replay::describe;
use crate::scrub::{self, Scrubbed, Secrets};

/// Each file a report holds, in the order the prompt lists them, with what
/// it holds.
pub(crate) const FILES: [(&str, &str); 6] = [
    ("report.md", "the summary, the form and the steps; start here"),
    (
        "events.jsonl",
        "every event in order, one JSON object a line; it replays as a flow test",
    ),
    ("state.txt", "the app's state at the start, each action's change, and at the end"),
    ("view.txt", "what the window showed at the start, each change, and at the end"),
    (
        "layout.txt",
        "where everything sat, in logical pixels, at the start, each change, and at the end",
    ),
    ("daemon.log", "the daemon calls, errors and log lines in the same period"),
];

/// The marks drawn on the window, in the folder only when there are some.
pub(crate) const DRAWING: (&str, &str) = (
    "drawing.json",
    "the marks drawn on the window: their paths in window points and the elements they cover",
);

/// The window as the engine drew it, saved by [`crate::write_window`].
pub(crate) const WINDOW: (&str, &str) = (
    "window.png",
    "a picture of the window when the report was taken, drawn by the app itself, with secrets covered",
);

/// The marks and pinned comments over `window.png`, as an SVG in window
/// points.
pub(crate) const MARKS: (&str, &str) = (
    "marks.svg",
    "the drawn marks (numbered like the Drawings list) and pinned comments (C1, C2) laid over window.png",
);

/// A video of a recording, when the app made one, under the first name
/// found.
pub(crate) const VIDEOS: [(&str, &str); 3] = [
    ("recording.mp4", "a video of the window during the recording, drawn by the app itself"),
    ("recording.webm", "a video of the window during the recording, drawn by the app itself"),
    ("recording.gif", "a video of the window during the recording, drawn by the app itself"),
];

/// The picture and video files of a report, in the order the prompt lists
/// them.
pub(crate) fn picture_files() -> impl Iterator<Item = (&'static str, &'static str)> {
    [WINDOW, MARKS].into_iter().chain(VIDEOS)
}

/// Ends the name of a report folder being written.
const PARTIAL: &str = ".partial";

/// The most reports with one name in one day before giving up.
const MAX_TRIES: usize = 1000;

/// The most words of "what happened" in the folder's name.
const SLUG_WORDS: usize = 6;

static WRITES: AtomicU64 = AtomicU64::new(0);

/// Saves `report` in `reports` as `<date>-<slug>/`, where the date is the
/// day it started (UTC) and the slug is the first words of what happened.
/// A name already taken gets `-2`, `-3` and so on. Returns the folder.
///
/// # Errors
///
/// [`WriteError`] when the reports folder can't be made, a file can't be
/// written or the folder can't be moved into place. Nothing is left
/// half-written.
pub fn write(reports: &Path, report: &Scrubbed) -> Result<PathBuf, WriteError> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(reports)
        .map_err(|source| WriteError::CreateReports {
            path: reports.to_owned(),
            source,
        })?;

    let captured = report.captured();
    let name = format!(
        "{}-{}",
        date(captured.context.started_unix_ms),
        slug(&report.form().happened, captured.kind)
    );
    let files = render(captured, report.form()).map_err(|source| WriteError::WriteFile {
        file: DRAWING.0,
        path: reports.to_owned(),
        source,
    })?;

    let temp = reports.join(format!(
        ".{name}.{}-{}{PARTIAL}",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    let published = fill(&temp, &files).and_then(|()| publish(reports, &temp, &name));
    if published.is_err() {
        // Already moved or never made are both fine.
        let _ = fs::remove_dir_all(&temp);
    }
    published
}

/// Saves `form` into the report already written in `dir`, scrubbed of
/// what `secrets` seeds: its `report.md` is made again from the report's
/// `events.jsonl` and the new form, and replaces the old one whole. This
/// lets a report be saved the moment it stops (so `humata report stop`
/// can answer with its folder, and a crash while the form is open loses
/// nothing) and the form be filled in after. The folder keeps its name.
///
/// # Errors
///
/// [`FormError`] when `events.jsonl` can't be read back or `report.md`
/// can't be replaced; the old `report.md` is kept.
pub fn fill_form(dir: &Path, form: Form, secrets: &Secrets) -> Result<(), FormError> {
    let events = dir.join("events.jsonl");
    let text = fs::read_to_string(&events).map_err(|source| FormError::ReadEvents {
        path: events.clone(),
        source,
    })?;
    let captured = jsonl::read_events(&text).map_err(|source| FormError::Events { path: events, source })?;
    let form = scrub::scrub_form_alone(form, secrets);
    let extra: Vec<(&'static str, &'static str)> = picture_files()
        .filter(|(name, _)| dir.join(name).is_file() && *name != MARKS.0)
        .collect();
    let has_window = extra.iter().any(|(name, _)| *name == WINDOW.0);

    // The overlay goes first: report.md lists it only when it exists.
    let marks = dir.join(MARKS.0);
    let overlay = picture::marks_svg(&form, has_window);
    match &overlay {
        None => match fs::remove_file(&marks) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(source) => return Err(FormError::Write { path: marks, source }),
        },
        Some(svg) => replace_file(dir, MARKS.0, svg.as_bytes())
            .map_err(|source| FormError::Write { path: marks, source })?,
    }
    let mut extra = extra;
    if overlay.is_some() {
        extra.insert(usize::from(has_window), MARKS);
    }
    let text = report_md(&captured, &form, &extra);

    let drawing = dir.join(DRAWING.0);
    if form.drawing.strokes.is_empty() {
        match fs::remove_file(&drawing) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(source) => return Err(FormError::Write { path: drawing, source }),
        }
    } else {
        let json = drawing_json(&form.drawing).map_err(|source| FormError::Write {
            path: drawing.clone(),
            source,
        })?;
        replace_file(dir, DRAWING.0, json.as_bytes()).map_err(|source| FormError::Write { path: drawing, source })?;
    }
    replace_file(dir, "report.md", text.as_bytes()).map_err(|source| FormError::Write {
        path: dir.join("report.md"),
        source,
    })
}

/// `drawing` as pretty JSON.
fn drawing_json(drawing: &Drawing) -> std::io::Result<String> {
    let mut json = serde_json::to_string_pretty(drawing).map_err(std::io::Error::other)?;
    json.push('\n');
    Ok(json)
}

/// Replaces `dir/name` whole with `text`, readable only by this user; the
/// old file is kept when it can't be.
pub(crate) fn replace_file(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    let target = dir.join(name);
    let temp = dir.join(format!(
        ".{name}.{}-{}{PARTIAL}",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| {
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        out.set_permissions(Permissions::from_mode(0o600))?;
        out.write_all(bytes)?;
        out.sync_all()?;
        fs::rename(&temp, &target)?;
        File::open(dir)?.sync_all()
    })();
    if written.is_err() {
        // Already renamed or never made are both fine.
        let _ = fs::remove_file(&temp);
    }
    written
}

/// Makes `temp` and writes every file into it, each synced to disk.
fn fill(temp: &Path, files: &[(&'static str, String)]) -> Result<(), WriteError> {
    let failed = |file: &'static str| {
        let path = temp.to_owned();
        move |source| WriteError::WriteFile { file, path, source }
    };
    DirBuilder::new()
        .mode(0o700)
        .create(temp)
        .and_then(|()| fs::set_permissions(temp, Permissions::from_mode(0o700)))
        .map_err(failed("folder"))?;
    for &(file, ref text) in files {
        let path = temp.join(file);
        (|| {
            let mut out = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            // The mode above is narrowed by the umask; set it exactly.
            out.set_permissions(Permissions::from_mode(0o600))?;
            out.write_all(text.as_bytes())?;
            out.sync_all()
        })()
        .map_err(failed(file))?;
    }
    File::open(temp)
        .and_then(|folder| folder.sync_all())
        .map_err(failed("folder"))
}

/// Renames `temp` to the first free `name`, `name-2`, ... in `reports`.
fn publish(reports: &Path, temp: &Path, name: &str) -> Result<PathBuf, WriteError> {
    for attempt in 1..=MAX_TRIES {
        let target = if attempt == 1 {
            reports.join(name)
        } else {
            reports.join(format!("{name}-{attempt}"))
        };
        if fs::symlink_metadata(&target).is_ok() {
            continue;
        }
        match fs::rename(temp, &target) {
            Ok(()) => {
                File::open(reports)
                    .and_then(|folder| folder.sync_all())
                    .map_err(|source| WriteError::Publish {
                        path: target.clone(),
                        source,
                    })?;
                return Ok(target);
            }
            // Taken between the check and the rename.
            Err(error) if matches!(error.kind(), ErrorKind::AlreadyExists | ErrorKind::DirectoryNotEmpty) => {}
            Err(source) => return Err(WriteError::Publish { path: target, source }),
        }
    }
    Err(WriteError::NoFreeName {
        reports: reports.to_owned(),
        name: name.to_owned(),
        tries: MAX_TRIES,
    })
}

/// The day `unix_ms` falls on, in UTC, as `YYYY-MM-DD`.
fn date(unix_ms: u64) -> String {
    // Howard Hinnant's days-to-civil algorithm.
    let days = i64::try_from(unix_ms / 86_400_000).unwrap_or(i64::MAX / 2);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The first words of `happened`, lower case, joined by `-`, or the kind
/// of report when it has no ASCII words.
fn slug(happened: &str, kind: Kind) -> String {
    let words: Vec<String> = happened
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(SLUG_WORDS)
        .map(str::to_ascii_lowercase)
        .collect();
    if words.is_empty() {
        kind.to_string()
    } else {
        words.join("-")
    }
}

/// Every file's text.
fn render(captured: &Captured, form: &Form) -> std::io::Result<Vec<(&'static str, String)>> {
    let mut files = vec![
        ("report.md", report_md(captured, form, &[])),
        ("events.jsonl", jsonl::write(captured)),
        ("state.txt", trees_txt(captured, Tree::State)),
        ("view.txt", trees_txt(captured, Tree::View)),
        ("layout.txt", trees_txt(captured, Tree::Layout)),
        ("daemon.log", daemon_log(captured)),
    ];
    if !form.drawing.strokes.is_empty() {
        files.push((DRAWING.0, drawing_json(&form.drawing)?));
    }
    Ok(files)
}

fn seconds(ms: u64) -> String {
    format!("{}.{:03} s", ms / 1000, ms % 1000)
}

/// `text` as one markdown bullet's value: later lines indented under it.
fn bullet_text(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return "(not given)".to_owned();
    }
    text.lines().collect::<Vec<_>>().join("\n  ")
}

fn report_md(captured: &Captured, form: &Form, extra: &[(&str, &str)]) -> String {
    let context = &captured.context;
    let mut md = String::new();
    let title = form.happened.lines().next().map(str::trim).unwrap_or_default();
    let title = if title.is_empty() { "no description" } else { title };
    let _ = writeln!(md, "# Humata bug report ({}): {title}\n", captured.kind);

    let _ = writeln!(md, "## Summary\n");
    let _ = write!(md, "- **Kind:** {}, started {} UTC", captured.kind, date(context.started_unix_ms));
    if captured.kind == Kind::Recording {
        let _ = write!(md, ", {} long", seconds(captured.end_ms));
    }
    md.push('\n');
    let _ = writeln!(md, "- **What happened:** {}", bullet_text(&form.happened));
    let _ = writeln!(md, "- **What I expected:** {}", bullet_text(&form.expected));
    let _ = writeln!(
        md,
        "- **App:** {} on {}, window {}x{}",
        context.app_version, context.os, context.window.width, context.window.height
    );
    let list = |items: Vec<String>| {
        if items.is_empty() {
            "none".to_owned()
        } else {
            items.join(", ")
        }
    };
    let daemons = context
        .daemons
        .iter()
        .map(|daemon| format!("{} {}", daemon.machine, daemon.version))
        .collect();
    let _ = writeln!(md, "- **Daemons:** {}", list(daemons));
    let machines = context
        .machines
        .iter()
        .map(|machine| format!("{} ({}, {})", machine.name, machine.kind, machine.state))
        .collect();
    let _ = writeln!(md, "- **Machines:** {}", list(machines));
    let sessions = context
        .sessions
        .iter()
        .map(|session| {
            format!(
                "{} `{}` ({} on {}, {})",
                session.title, session.id, session.channel, session.machine, session.agent
            )
        })
        .collect();
    let _ = writeln!(md, "- **Sessions:** {}", list(sessions));
    if captured.kind == Kind::Recording {
        let count = |pick: fn(&Event) -> bool| captured.events.iter().filter(|event| pick(event)).count();
        let _ = writeln!(
            md,
            "- **Events:** {} interactions, {} actions, {} view changes, {} layout changes, \
             {} daemon calls, {} daemon errors, {} log lines",
            count(|event| matches!(event, Event::Interaction(_))),
            count(|event| matches!(event, Event::Action(_))),
            count(|event| matches!(event, Event::View(_))),
            count(|event| matches!(event, Event::Layout(_))),
            count(|event| matches!(event, Event::Rpc(_))),
            count(|event| matches!(event, Event::DaemonError(_))),
            count(|event| matches!(event, Event::Log(_))),
        );
    }
    if captured.dropped > 0 {
        let _ = writeln!(
            md,
            "- **Full:** the recording reached its limit, so its oldest {} events were folded \
             into the start, which is the app as it was at {}",
            captured.dropped,
            seconds(captured.start_ms)
        );
    }
    let _ = writeln!(md, "- **Comments:** {} pinned to elements", form.comments.len());
    let strokes = &form.drawing.strokes;
    if !strokes.is_empty() {
        let _ = writeln!(md, "- **Drawings:** {} marks on the window", strokes.len());
    }
    md.push('\n');

    if !form.comments.is_empty() {
        let _ = writeln!(md, "## Comments\n");
        for comment in &form.comments {
            let _ = writeln!(md, "- `{}`: {}", comment.element, bullet_text(&comment.text));
        }
        md.push('\n');
    }

    if !strokes.is_empty() {
        let _ = writeln!(md, "## Drawings\n");
        for (number, stroke) in strokes.iter().enumerate() {
            let _ = writeln!(md, "{}. {}", number + 1, stroke.describe());
            let _ = writeln!(md, "   {}", path_line(&stroke.points));
        }
        md.push('\n');
    }

    let interactions: Vec<String> = captured
        .events
        .iter()
        .filter_map(|event| match event {
            Event::Interaction(interaction) => Some(format!(
                "{} {}",
                seconds(interaction.at_ms),
                describe(&interaction.gesture)
            )),
            _ => None,
        })
        .collect();
    if !interactions.is_empty() {
        let _ = writeln!(md, "## Steps\n");
        for (number, line) in interactions.iter().enumerate() {
            let _ = writeln!(md, "{}. {line}", number + 1);
        }
        md.push('\n');

        let script = captured.flow_script();
        let _ = writeln!(md, "## Replay as a flow\n");
        let _ = writeln!(md, "```rust\n{}```\n", script.to_rust("report", title));
        if !script.skipped.is_empty() {
            let _ = writeln!(md, "Left out of the flow:\n");
            for skipped in &script.skipped {
                let _ = writeln!(md, "- {} {}: {}", seconds(skipped.at_ms), skipped.what, skipped.reason);
            }
            md.push('\n');
        }
    }

    let _ = writeln!(md, "## Files\n");
    for (file, holds) in FILES {
        let _ = writeln!(md, "- `{file}`: {holds}");
    }
    if !strokes.is_empty() {
        let _ = writeln!(md, "- `{}`: {}", DRAWING.0, DRAWING.1);
    }
    for (file, holds) in extra {
        let _ = writeln!(md, "- `{file}`: {holds}");
    }
    md
}

/// How many points a stroke has and the box they lie in, in window points.
fn path_line(points: &[crate::form::Point]) -> String {
    let Some(first) = points.first() else {
        return "no points".to_owned();
    };
    let (mut left, mut top, mut right, mut bottom) = (first.x, first.y, first.x, first.y);
    for point in points {
        left = left.min(point.x);
        right = right.max(point.x);
        top = top.min(point.y);
        bottom = bottom.max(point.y);
    }
    format!(
        "{} points, within ({left}, {top}) to ({right}, {bottom}) in window points",
        points.len()
    )
}

#[derive(Clone, Copy)]
enum Tree {
    State,
    View,
    Layout,
}

impl Tree {
    fn name(self) -> &'static str {
        match self {
            Self::State => "state",
            Self::View => "view",
            Self::Layout => "layout",
        }
    }

    fn of(self, trees: &crate::event::Trees) -> &str {
        match self {
            Self::State => &trees.state,
            Self::View => &trees.view,
            Self::Layout => &trees.layout,
        }
    }
}

fn indented(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        out.push_str("    ");
        out.push_str(line);
        out.push('\n');
    }
    if out.is_empty() {
        out.push_str("    (empty)\n");
    }
    out
}

fn trees_txt(captured: &Captured, tree: Tree) -> String {
    let name = tree.name();
    let mut out = String::new();
    if captured.kind == Kind::Snapshot {
        let _ = writeln!(out, "{name}");
        out.push_str(&indented(tree.of(&captured.start)));
        return out;
    }

    let _ = write!(out, "{name} at start ({}", seconds(captured.start_ms));
    if captured.dropped > 0 {
        let _ = write!(out, ", after {} older events were folded in", captured.dropped);
    }
    out.push_str(")\n");
    out.push_str(&indented(tree.of(&captured.start)));

    let mut last_interaction: Option<String> = None;
    for event in &captured.events {
        let (at_ms, heading, hunks) = match (tree, event) {
            (_, Event::Interaction(interaction)) => {
                last_interaction = Some(describe(&interaction.gesture));
                continue;
            }
            (Tree::State, Event::Action(step)) => (step.at_ms, step.action.clone(), &step.change),
            (Tree::View, Event::View(change)) | (Tree::Layout, Event::Layout(change)) => {
                let after = last_interaction
                    .as_deref()
                    .map_or_else(|| "changed".to_owned(), |what| format!("after {what}"));
                (change.at_ms, after, &change.change)
            }
            _ => continue,
        };
        let _ = writeln!(out, "\n{} {heading}", seconds(at_ms));
        if hunks.is_empty() {
            out.push_str("    (unchanged)\n");
        } else {
            out.push_str(&change::render(hunks, "    "));
        }
    }

    let _ = writeln!(out, "\n{name} at end ({})", seconds(captured.end_ms));
    out.push_str(&indented(tree.of(&captured.end)));
    out
}

fn daemon_log(captured: &Captured) -> String {
    let mut out = String::new();
    for event in &captured.events {
        match event {
            Event::Rpc(rpc) => {
                let _ = write!(
                    out,
                    "{} {} rpc {} {} in {} ms",
                    seconds(rpc.at_ms),
                    rpc.machine,
                    rpc.method,
                    rpc.status,
                    rpc.duration_ms
                );
                if let Holds::Terminal { terminal } = &rpc.holds {
                    let _ = write!(out, " (terminal {terminal:?})");
                }
                out.push('\n');
                for (label, payload) in [("request", &rpc.request), ("response", &rpc.response)] {
                    if let Some(payload) = payload {
                        let _ = writeln!(out, "    {label}:");
                        for line in payload.lines() {
                            let _ = writeln!(out, "        {line}");
                        }
                    }
                }
            }
            Event::DaemonError(error) => {
                let _ = write!(out, "{} {} error {}", seconds(error.at_ms), error.machine, error.code);
                if let Some(method) = &error.method {
                    let _ = write!(out, " in {method}");
                }
                let _ = writeln!(out, ": {}", error.message);
            }
            Event::Log(line) => {
                let _ = writeln!(out, "{} {} log {}", seconds(line.at_ms), line.machine, line.line);
            }
            Event::Interaction(interaction) => {
                // Shown so each call can be read against what caused it.
                if !matches!(interaction.gesture, Gesture::Scroll { .. }) {
                    let _ = writeln!(
                        out,
                        "{} app {}",
                        seconds(interaction.at_ms),
                        describe(&interaction.gesture)
                    );
                }
            }
            Event::Action(_) | Event::View(_) | Event::Layout(_) => {}
        }
    }
    if out.is_empty() {
        out.push_str("(no daemon calls, errors or log lines in this report)\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_dated_by_the_utc_day_the_report_started() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_794_652_200_000), "2026-11-14");
        assert_eq!(date(1_709_251_199_000), "2024-02-29");
    }

    #[test]
    fn a_slug_is_safe_as_a_folder_name() {
        let slug = slug("The Add button / did nothing!! after ../../paste", Kind::Recording);
        assert!(slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        assert!(!slug.is_empty());
        assert_eq!(super::slug("دکمه کار نکرد", Kind::Snapshot), "snapshot");
    }
}
