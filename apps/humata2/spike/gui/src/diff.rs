//! Screen 2: one diff across three repositories, 20,000 lines in all, many
//! of them long. Drawn with GPUI's virtualised `list`, which measures each
//! row, so wrapped lines take as many rows of text as they need.

use gpui_kit::prelude::*;
use gpui_kit::*;

/// How many lines the generated diff has.
pub const LINES: usize = 20_000;

const REPOS: [&str; 3] = ["atmosphere", "humata-daemon", "tca-rust"];
const DIRS: [&str; 6] = ["src", "src/view", "src/store", "crates/core/src", "tests", "apps/cli/src"];
const STEMS: [&str; 12] = [
    "session", "channel", "board", "terminal", "diff", "sidebar", "reducer", "effect", "store",
    "runner", "layout", "inbox",
];
const WORDS: [&str; 24] = [
    "let", "state", "self", "match", "action", "return", "Some", "None", "window", "cx",
    "notify", "rows", "index", "range", "len", "visible", "selected", "expanded", "into",
    "clone", "update", "render", "offset", "height",
];

/// What a line of the diff is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Repo,
    File,
    Hunk,
    Context,
    Added,
    Removed,
}

/// One line of the diff.
#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: SharedString,
}

/// A small xorshift generator, so every run makes the same diff.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A number in `low..high`, or `low` when the range is empty.
    fn range(&mut self, low: usize, high: usize) -> usize {
        let span = high.saturating_sub(low).max(1) as u64;
        low + (self.next() % span) as usize
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.range(0, items.len())]
    }
}

/// The diff's lines, the same every run: exactly `lines` of them.
pub fn generate(lines: usize) -> Vec<DiffLine> {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut out = Vec::with_capacity(lines);
    let per_repo = lines.div_ceil(REPOS.len());
    for (repo_index, repo) in REPOS.iter().enumerate() {
        let repo_end = ((repo_index + 1) * per_repo).min(lines);
        if out.len() >= repo_end {
            continue;
        }
        out.push(DiffLine {
            kind: LineKind::Repo,
            old: None,
            new: None,
            text: format!("{repo} — {} files changed", rng.range(20, 90)).into(),
        });
        let mut file_index = 0;
        while out.len() < repo_end {
            let path = format!(
                "{}/{}_{file_index}.rs",
                rng.pick(&DIRS),
                rng.pick(&STEMS)
            );
            file_index += 1;
            push_file(&mut rng, &mut out, &path, repo_end);
        }
    }
    out.truncate(lines);
    out
}

fn push_file(rng: &mut Rng, out: &mut Vec<DiffLine>, path: &str, end: usize) {
    out.push(DiffLine {
        kind: LineKind::File,
        old: None,
        new: None,
        text: path.to_owned().into(),
    });
    let mut old = rng.range(1, 400) as u32;
    let mut new = old;
    for _ in 0..rng.range(1, 6) {
        if out.len() >= end {
            return;
        }
        let body = rng.range(8, 60);
        out.push(DiffLine {
            kind: LineKind::Hunk,
            old: None,
            new: None,
            text: format!("@@ -{old},{body} +{new},{body} @@ fn {}()", rng.pick(&STEMS)).into(),
        });
        for _ in 0..body {
            if out.len() >= end {
                return;
            }
            let roll = rng.range(0, 100);
            let kind = if roll < 55 {
                LineKind::Context
            } else if roll < 80 {
                LineKind::Added
            } else {
                LineKind::Removed
            };
            let (old_no, new_no) = match kind {
                LineKind::Context => {
                    old += 1;
                    new += 1;
                    (Some(old), Some(new))
                }
                LineKind::Added => {
                    new += 1;
                    (None, Some(new))
                }
                _ => {
                    old += 1;
                    (Some(old), None)
                }
            };
            out.push(DiffLine {
                kind,
                old: old_no,
                new: new_no,
                text: code_line(rng).into(),
            });
        }
        let gap = rng.range(10, 80) as u32;
        old += gap;
        new += gap;
    }
}

/// A line of made-up code: usually short, about one in twelve (8%) 180 to 420
/// characters long.
fn code_line(rng: &mut Rng) -> String {
    let target = if rng.range(0, 100) < 8 {
        rng.range(180, 420)
    } else {
        rng.range(4, 90)
    };
    let mut line = " ".repeat(4 * rng.range(0, 4));
    while line.len() < target {
        line.push_str(rng.pick(&WORDS));
        line.push_str(match rng.range(0, 8) {
            0 => ".",
            1 => "(",
            2 => ", ",
            3 => " = ",
            _ => " ",
        });
    }
    line
}

/// The diff screen.
pub struct DiffScreen {
    lines: Vec<DiffLine>,
    list: ListState,
    wrap: bool,
    last_top: usize,
    stalled: u32,
}

impl DiffScreen {
    pub fn new(wrap: bool) -> Self {
        let lines = generate(LINES);
        let list = ListState::new(lines.len(), ListAlignment::Top, px(400.));
        Self {
            lines,
            list,
            wrap,
            last_top: 0,
            stalled: 0,
        }
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn wraps(&self) -> bool {
        self.wrap
    }

    pub fn set_wrap(&mut self, wrap: bool, cx: &mut Context<Self>) {
        if self.wrap != wrap {
            self.wrap = wrap;
            // Every row's height may change.
            self.list.remeasure();
            cx.notify();
        }
    }

    /// Scrolls `step` further down, for the scroll benchmark. True once the
    /// last line is in view, or scrolling has stopped moving.
    pub fn bench_step(&mut self, step: Pixels, cx: &mut Context<Self>) -> bool {
        if self.list.is_scrolled_to_end() == Some(true) {
            return true;
        }
        let top = self.list.logical_scroll_top().item_ix;
        if top + 1 >= self.lines.len() {
            return true;
        }
        if top == self.last_top {
            self.stalled += 1;
        } else {
            self.stalled = 0;
            self.last_top = top;
        }
        // A step shorter than one tall wrapped row leaves the top row the
        // same for a while, so only many still ticks count as stuck.
        if self.stalled > 120 {
            return true;
        }
        self.list.scroll_by(step);
        cx.notify();
        false
    }

    fn render_line(&self, index: usize) -> AnyElement {
        let Some(line) = self.lines.get(index) else {
            return div().into_any_element();
        };
        match line.kind {
            LineKind::Repo => div()
                .px_3()
                .pt_4()
                .pb_1()
                .text_size(px(15.))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(0xf0f3f8))
                .bg(rgb(0x14171d))
                .child(line.text.clone())
                .into_any_element(),
            LineKind::File => div()
                .px_3()
                .py_1()
                .border_t_1()
                .border_color(rgb(0x30363f))
                .bg(rgb(0x222832))
                .text_color(rgb(0xd5dbe5))
                .font_weight(FontWeight::SEMIBOLD)
                .child(line.text.clone())
                .into_any_element(),
            LineKind::Hunk => div()
                .px_3()
                .bg(rgb(0x1d2a3a))
                .text_color(rgb(0x7fa7d8))
                .child(line.text.clone())
                .into_any_element(),
            LineKind::Context | LineKind::Added | LineKind::Removed => {
                let (sign, bg, fg) = match line.kind {
                    LineKind::Added => ("+", rgb(0x183223), rgb(0xb5e8c3)),
                    LineKind::Removed => ("-", rgb(0x3a1d22), rgb(0xf2b8bf)),
                    _ => (" ", rgb(0x1b1f27), rgb(0xc3cad6)),
                };
                let number = |n: Option<u32>| {
                    div()
                        .w(px(52.))
                        .flex_shrink_0()
                        .pr_2()
                        .text_right()
                        .text_color(rgb(0x5d6675))
                        .child(n.map(|n| n.to_string()).unwrap_or_default())
                };
                let text = div()
                    .flex_1()
                    .min_w(px(0.))
                    .text_color(fg)
                    .child(line.text.clone());
                let text = if self.wrap {
                    text
                } else {
                    text.whitespace_nowrap().overflow_hidden()
                };
                div()
                    .flex()
                    .flex_row()
                    .bg(bg)
                    .child(number(line.old))
                    .child(number(line.new))
                    .child(div().w(px(16.)).flex_shrink_0().text_color(fg).child(sign))
                    .child(text)
                    .into_any_element()
            }
        }
    }
}

impl Render for DiffScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let wrap = self.wrap;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1b1f27))
            .text_color(rgb(0xe6e9ef))
            .text_size(px(12.))
            .font_family(crate::terminal::MONOSPACE)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .border_b_1()
                    .border_color(rgb(0x2d3340))
                    .child(
                        div()
                            .id("wrap-toggle")
                            .test_support()
                            .role(Role::Button)
                            .aria_label("Wrap lines")
                            .aria_selected(wrap)
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(if wrap { rgb(0x2f4a72) } else { rgb(0x2d3340) })
                            .cursor_pointer()
                            .child(if wrap { "Wrap: on" } else { "Wrap: off" })
                            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                let wrap = !this.wrap;
                                this.set_wrap(wrap, cx);
                            })),
                    )
                    .child(
                        div()
                            .text_color(rgb(0x8a93a3))
                            .child(format!("{} lines in {} repositories", self.lines.len(), REPOS.len())),
                    ),
            )
            .child(
                div().flex_1().min_h(px(0.)).child(
                    list(
                        self.list.clone(),
                        cx.processor(|this, index: usize, _window, _cx| this.render_line(index)),
                    )
                    .size_full(),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::std::prelude::v1::test]
    fn the_diff_is_the_same_every_time_and_exactly_as_long_as_asked() {
        let first = generate(LINES);
        let second = generate(LINES);
        assert_eq!(first.len(), LINES);
        assert!(first.iter().zip(&second).all(|(a, b)| a.text == b.text && a.kind == b.kind));
    }

    #[::std::prelude::v1::test]
    fn the_diff_spans_three_repositories_with_long_lines_and_both_signs() {
        let lines = generate(LINES);
        let count = |kind| lines.iter().filter(|line| line.kind == kind).count();
        assert_eq!(count(LineKind::Repo), REPOS.len());
        assert!(count(LineKind::File) > 50);
        assert!(count(LineKind::Added) > 0 && count(LineKind::Removed) > 0);
        assert!(lines.iter().any(|line| line.text.len() >= 180));
    }
}
