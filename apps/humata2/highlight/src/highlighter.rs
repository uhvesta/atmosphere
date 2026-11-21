//! Syntax highlighting backed by `gpui-component`'s syntect + two-face
//! engine, with per-document caching so UI frames never re-lex unchanged code.

use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::ops::Range;
use std::sync::{Arc, LazyLock, Mutex};

use gpui_kit::base::text::CodeBlock;
use gpui_kit::component::highlighter::{HighlightTheme, highlight_code};
use gpui_kit::HighlightStyle;

use crate::Language;

/// Lines longer than this (e.g. minified bundles) are skipped so highlighting
/// never stalls on degenerate inputs.
const MAX_LINE_BYTES: usize = 1024;

/// Maximum number of cached highlighted documents kept in memory.
const CACHE_CAPACITY: usize = 64;

/// Maximum total lines highlighted in one full-file pass; files larger than
/// this still highlight their first `MAX_FILE_LINES` lines cleanly and leave
/// the tail plain rather than stalling.
const MAX_FILE_LINES: usize = 8_000;

/// Per-line highlight spans: `lines[i]` holds byte ranges relative to line `i`.
pub type LineHighlights = Vec<Vec<(Range<usize>, HighlightStyle)>>;

/// The default light or dark highlight theme from `gpui-component`.
#[must_use]
pub fn theme(dark: bool) -> Arc<HighlightTheme> {
    if dark {
        HighlightTheme::default_dark()
    } else {
        HighlightTheme::default_light()
    }
}

/// Highlights `text` as `language` in light or dark theme, returning only spans
/// that carry a visible style (colour, weight, or italic/underline).
#[must_use]
pub fn highlight_text(
    language: Language,
    text: &str,
    dark: bool,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let Some(token) = language.syntax_token() else {
        return Vec::new();
    };
    if text.is_empty() {
        return Vec::new();
    }
    let palette = theme(dark);
    highlight_code(token, text, palette.as_ref())
        .into_iter()
        .filter(|(range, style)| !range.is_empty() && is_styled(style))
        .collect()
}

/// Highlights a single line of code.
#[must_use]
pub fn highlight_line(
    language: Language,
    line: &str,
    dark: bool,
) -> Vec<(Range<usize>, HighlightStyle)> {
    if line.is_empty() || line.len() > MAX_LINE_BYTES {
        return Vec::new();
    }
    highlight_text(language, line, dark)
}

/// Highlights `lines` together in a single syntect pass so multi-line strings
/// and block comments carry across lines, then splits the spans into per-line
/// byte ranges.
#[must_use]
pub fn highlight_lines(language: Language, lines: &[&str], dark: bool) -> LineHighlights {
    let mut per_line = vec![Vec::new(); lines.len()];
    let Some(token) = language.syntax_token() else {
        return per_line;
    };
    if lines.is_empty() {
        return per_line;
    }

    let limit = lines.len().min(MAX_FILE_LINES);
    let mut joined = String::new();
    let mut offsets: Vec<(usize, usize)> = Vec::with_capacity(limit);
    for &line in &lines[..limit] {
        let start = joined.len();
        if line.len() <= MAX_LINE_BYTES {
            joined.push_str(line);
        }
        let end = joined.len();
        offsets.push((start, end));
        joined.push('\n');
    }

    let palette = theme(dark);
    let spans = highlight_code(token, &joined, palette.as_ref());
    let mut line_ix = 0;
    for (range, style) in spans {
        if range.is_empty() || !is_styled(&style) {
            continue;
        }
        while line_ix < offsets.len() && offsets[line_ix].1 <= range.start {
            line_ix += 1;
        }
        let mut cursor = line_ix;
        while cursor < offsets.len() {
            let (line_start, line_end) = offsets[cursor];
            if line_start >= range.end {
                break;
            }
            let start = range.start.max(line_start);
            let end = range.end.min(line_end);
            if start < end {
                let local = (start - line_start)..(end - line_start);
                let line = lines[cursor];
                if line.is_char_boundary(local.start) && line.is_char_boundary(local.end) {
                    per_line[cursor].push((local, style));
                }
            }
            cursor += 1;
        }
    }
    per_line
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct CacheKey {
    language: Language,
    dark: bool,
    hash: u64,
    line_count: usize,
}

static LINE_CACHE: LazyLock<Mutex<VecDeque<(CacheKey, Arc<LineHighlights>)>>> =
    LazyLock::new(|| Mutex::new(VecDeque::with_capacity(CACHE_CAPACITY)));

/// Cached version of [`highlight_lines`]: repeated frames for the same file
/// content, language and theme return the cached `Arc` in O(1) without
/// re-running syntect.
#[must_use]
pub fn highlight_lines_cached(
    language: Language,
    lines: &[&str],
    dark: bool,
) -> Arc<LineHighlights> {
    if language == Language::Plain || lines.is_empty() {
        return Arc::new(vec![Vec::new(); lines.len()]);
    }
    let mut hasher = DefaultHasher::new();
    for line in lines {
        line.hash(&mut hasher);
        0xff_u8.hash(&mut hasher);
    }
    let key = CacheKey {
        language,
        dark,
        hash: hasher.finish(),
        line_count: lines.len(),
    };
    if let Ok(cache) = LINE_CACHE.lock()
        && let Some((_, hit)) = cache.iter().find(|(k, _)| *k == key)
    {
        return hit.clone();
    }
    let computed = Arc::new(highlight_lines(language, lines, dark));
    if let Ok(mut cache) = LINE_CACHE.lock() {
        if cache.len() >= CACHE_CAPACITY {
            cache.pop_front();
        }
        cache.push_back((key, computed.clone()));
    }
    computed
}

/// Highlights a Markdown fenced code block using the shared detector: fence
/// info string first, then shebang/modeline fallback when the fence is
/// unlabelled.
#[must_use]
pub fn highlight_code_block(
    lang: Option<&str>,
    code: &str,
    dark: bool,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let detected = lang
        .map(Language::from_fence)
        .filter(|l| *l != Language::Plain)
        .unwrap_or_else(|| Language::from_content(code));
    highlight_text(detected, code, dark)
}

/// Type alias matching `gpui_kit::base::TextView::shared_code_block_highlighter`.
pub type SharedCodeBlockHighlighter =
    dyn Fn(&CodeBlock) -> Vec<(Range<usize>, HighlightStyle)> + Send + Sync;

static LIGHT_BLOCK_HIGHLIGHTER: LazyLock<Arc<SharedCodeBlockHighlighter>> = LazyLock::new(|| {
    Arc::new(|block: &CodeBlock| {
        let lang = block.lang();
        highlight_code_block(lang.as_deref(), block.code().as_ref(), false)
    })
});

static DARK_BLOCK_HIGHLIGHTER: LazyLock<Arc<SharedCodeBlockHighlighter>> = LazyLock::new(|| {
    Arc::new(|block: &CodeBlock| {
        let lang = block.lang();
        highlight_code_block(lang.as_deref(), block.code().as_ref(), true)
    })
});

/// Returns a stable `Arc` code-block highlighter for `TextView` so unchanged
/// Markdown code blocks reuse their cached highlights across frames.
#[must_use]
pub fn shared_code_block_highlighter(dark: bool) -> Arc<SharedCodeBlockHighlighter> {
    if dark {
        DARK_BLOCK_HIGHLIGHTER.clone()
    } else {
        LIGHT_BLOCK_HIGHLIGHTER.clone()
    }
}

/// Merges sorted, non-overlapping syntax `base` spans with an optional
/// `overlay` span (such as an inline diff word highlight), preserving both
/// foreground syntax colours and the overlay's background colour.
#[must_use]
pub fn merge_overlay(
    base: Vec<(Range<usize>, HighlightStyle)>,
    overlay: Option<(Range<usize>, HighlightStyle)>,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let Some((ov_range, ov_style)) = overlay else {
        return base;
    };
    if ov_range.is_empty() {
        return base;
    }
    if base.is_empty() {
        return vec![(ov_range, ov_style)];
    }

    let mut out = Vec::with_capacity(base.len() + 2);
    let mut ov_cursor = ov_range.start;

    for (range, style) in base {
        if range.end <= ov_range.start || range.start >= ov_range.end {
            if range.start >= ov_range.end && ov_cursor < ov_range.end {
                out.push((ov_cursor..ov_range.end, ov_style));
                ov_cursor = ov_range.end;
            }
            out.push((range, style));
            continue;
        }
        // Part of `range` before `ov_range`.
        if range.start < ov_range.start {
            out.push((range.start..ov_range.start, style));
        }
        let overlap_start = range.start.max(ov_range.start);
        let overlap_end = range.end.min(ov_range.end);
        // Uncovered gap of `ov_range` before `overlap_start`.
        if ov_cursor < overlap_start {
            out.push((ov_cursor..overlap_start, ov_style));
        }
        let mut combined = style;
        if ov_style.background_color.is_some() {
            combined.background_color = ov_style.background_color;
        }
        if ov_style.color.is_some() {
            combined.color = ov_style.color;
        }
        out.push((overlap_start..overlap_end, combined));
        ov_cursor = overlap_end;
        // Part of `range` after `ov_range`.
        if range.end > ov_range.end {
            out.push((ov_range.end..range.end, style));
        }
    }
    if ov_cursor < ov_range.end {
        out.push((ov_cursor..ov_range.end, ov_style));
    }
    out
}

fn is_styled(style: &HighlightStyle) -> bool {
    style.color.is_some()
        || style.background_color.is_some()
        || style.font_weight.is_some()
        || style.font_style.is_some()
        || style.underline.is_some()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn distinct_colors(spans: &[(Range<usize>, HighlightStyle)]) -> usize {
        spans
            .iter()
            .filter_map(|(_, style)| style.color)
            .map(|c| {
                (
                    (c.h * 1000.0).round() as i32,
                    (c.s * 1000.0).round() as i32,
                    (c.l * 1000.0).round() as i32,
                )
            })
            .collect::<HashSet<_>>()
            .len()
    }

    const SAMPLES: &[(Language, &str, &str)] = &[
        (
            Language::Rust,
            "src/lib.rs",
            "pub fn add(a: i32, b: i32) -> i32 {\n    // sum\n    let label = \"ok\";\n    a + b + 1\n}\n",
        ),
        (
            Language::Go,
            "main.go",
            "package main\n\nfunc main() {\n\t// greet\n\tmsg := \"hello\"\n\tprintln(msg, 42)\n}\n",
        ),
        (
            Language::Python,
            "app.py",
            "def greet(name: str) -> str:\n    # greeting\n    return f\"hello {name}\" if True else \"\"\n",
        ),
        (
            Language::Toml,
            "Cargo.toml",
            "[package]\nname = \"demo\" # crate name\nversion = \"1.0.0\"\nedition = 2024\n",
        ),
        (
            Language::Yaml,
            "ci.yaml",
            "# workflow\nname: build\non: [push]\njobs:\n  test:\n    runs-on: ubuntu-latest\n    timeout: 15\n",
        ),
        (
            Language::Json,
            "package.json",
            "{\n  \"name\": \"humata\",\n  \"version\": 2,\n  \"private\": true\n}\n",
        ),
        (
            Language::Java,
            "App.java",
            "public class App {\n    // entry\n    public static void main(String[] args) {\n        int n = 42;\n        System.out.println(\"hi\");\n    }\n}\n",
        ),
        (
            Language::Kotlin,
            "Main.kt",
            "fun main() {\n    // greet\n    val msg = \"hello\"\n    val count = 3\n}\n",
        ),
        (
            Language::C,
            "main.c",
            "#include <stdio.h>\nint main(void) {\n    /* comment */\n    const char *s = \"hi\";\n    return 0;\n}\n",
        ),
        (
            Language::Cpp,
            "main.cpp",
            "#include <string>\nclass Box {\npublic:\n    // value\n    std::string name = \"box\";\n    int count = 7;\n};\n",
        ),
        (
            Language::Swift,
            "App.swift",
            "struct User {\n    // name\n    let name: String = \"Ada\"\n    let age: Int = 36\n}\n",
        ),
        (
            Language::TypeScript,
            "index.ts",
            "export function greet(name: string): string {\n  // hello\n  const n: number = 1;\n  return \"hi\";\n}\n",
        ),
        (
            Language::JavaScript,
            "index.js",
            "function greet(name) {\n  // hello\n  const n = 42;\n  return \"hi\";\n}\n",
        ),
        (
            Language::Shell,
            "run.sh",
            "#!/usr/bin/env bash\n# deploy\nif [ -n \"$HOME\" ]; then\n  echo \"ok\" 123\nfi\n",
        ),
        (
            Language::Sql,
            "schema.sql",
            "-- users table\nSELECT id, name FROM users WHERE active = 1 AND role = 'admin';\n",
        ),
        (
            Language::Terraform,
            "main.tf",
            "# bucket\nresource \"aws_s3_bucket\" \"logs\" {\n  bucket = \"humata-logs\"\n  force_destroy = true\n}\n",
        ),
        (
            Language::Markdown,
            "README.md",
            "# Title\n\nSome **bold** text and `inline` code with [a link](https://example.com).\n",
        ),
        (
            Language::Dockerfile,
            "Dockerfile",
            "# build image\nFROM rust:1.80 AS builder\nWORKDIR /app\nRUN echo \"ok\"\n",
        ),
        (
            Language::Makefile,
            "Makefile",
            "# build\nCC := clang\nall:\n\t@echo \"done\"\n",
        ),
        (
            Language::Protobuf,
            "events.proto",
            "syntax = \"proto3\";\n// event\nmessage Event {\n  string id = 1;\n  int32 count = 2;\n}\n",
        ),
        (
            Language::Starlark,
            "BUILD.bazel",
            "# rust target\nload(\"@rules_rust//rust:defs.bzl\", \"rust_library\")\nrust_library(\n    name = \"demo\",\n    srcs = [\"lib.rs\"],\n)\n",
        ),
    ];

    #[test]
    fn every_required_language_highlights_with_multiple_colours_in_light_and_dark() {
        for &(expected_lang, path, sample) in SAMPLES {
            assert_eq!(Language::from_path(path), expected_lang, "path `{path}`");
            for dark in [false, true] {
                let spans = highlight_text(expected_lang, sample, dark);
                let colors = distinct_colors(&spans);
                assert!(
                    colors >= 2,
                    "{expected_lang:?} ({path}, dark={dark}) produced only {colors} distinct colour(s): {spans:?}"
                );
            }
        }
    }

    #[test]
    fn highlight_lines_preserves_multiline_comments_and_splits_per_line() {
        let lines = ["/* start", "   middle */", "let x = 42;"];
        let per_line = highlight_lines(Language::Rust, &lines, true);
        assert_eq!(per_line.len(), 3);
        assert!(!per_line[0].is_empty(), "line 0 comment should highlight");
        assert!(!per_line[1].is_empty(), "line 1 comment continuation should highlight");
        assert!(distinct_colors(&per_line[2]) >= 2, "line 2 rust code should have >= 2 colours");
    }

    #[test]
    fn merge_overlay_keeps_syntax_colours_and_adds_background() {
        let kw = HighlightStyle {
            color: Some(gpui_kit::rgb(0x112233).into()),
            ..Default::default()
        };
        let num = HighlightStyle {
            color: Some(gpui_kit::rgb(0x445566).into()),
            ..Default::default()
        };
        let bg = HighlightStyle {
            background_color: Some(gpui_kit::rgb(0xaabbcc).into()),
            ..Default::default()
        };
        let merged = merge_overlay(vec![(0..3, kw), (8..10, num)], Some((2..9, bg)));
        assert_eq!(merged.len(), 5);
        assert_eq!(merged[0].0, 0..2);
        assert_eq!(merged[0].1.background_color, None);
        assert_eq!(merged[1].0, 2..3);
        assert_eq!(merged[1].1.color, kw.color);
        assert_eq!(merged[1].1.background_color, bg.background_color);
        assert_eq!(merged[2].0, 3..8);
        assert_eq!(merged[2].1.background_color, bg.background_color);
        assert_eq!(merged[3].0, 8..9);
        assert_eq!(merged[3].1.color, num.color);
        assert_eq!(merged[3].1.background_color, bg.background_color);
        assert_eq!(merged[4].0, 9..10);
        assert_eq!(merged[4].1.background_color, None);
    }

    #[test]
    fn markdown_code_block_highlighter_highlights_fences_in_light_and_dark() {
        let fences = [
            ("rust", "pub fn add(a: i32) -> i32 { let s = \"ok\"; a + 1 }\n"),
            ("go", "package main\nfunc main() { msg := \"hi\"; println(msg, 1) }\n"),
            ("python", "def f(x):\n    # note\n    return \"ok\" if True else 0\n"),
            ("toml", "[package]\nname = \"demo\" # comment\nversion = 1\n"),
            ("yaml", "# cfg\nname: build\ntimeout: 15\n"),
            ("java", "public class A { int n = 1; String s = \"x\"; }\n"),
            ("starlark", "# bazel\nload(\"//a:b.bzl\", \"r\")\nr(name = \"x\")\n"),
            ("bzl", "def rule(name):\n    # rule\n    return \"ok\"\n"),
            ("BUILD.bazel", "rust_library(name = \"demo\", srcs = [\"lib.rs\"])\n"),
            ("hcl", "resource \"aws_s3_bucket\" \"b\" { force_destroy = true }\n"),
            ("dockerfile", "FROM rust:1.80\nRUN echo \"ok\"\n"),
            ("proto", "syntax = \"proto3\";\nmessage M { string id = 1; }\n"),
        ];
        for (fence, code) in fences {
            for dark in [false, true] {
                let hl = shared_code_block_highlighter(dark);
                let block = CodeBlock::from_code(code, Some(fence));
                let spans = hl(&block);
                let colors = distinct_colors(&spans);
                assert!(
                    colors >= 2,
                    "markdown fence `{fence}` (dark={dark}) produced only {colors} colour(s): {spans:?}"
                );
            }
        }
    }

    #[test]
    fn cached_line_highlights_reuse_arc_across_frames() {
        let lines = ["pub fn main() {", "    let x = \"hello\";", "}"];
        let first = highlight_lines_cached(Language::Rust, &lines, true);
        let second = highlight_lines_cached(Language::Rust, &lines, true);
        assert!(Arc::ptr_eq(&first, &second), "repeated frame must hit cache");
    }
}
