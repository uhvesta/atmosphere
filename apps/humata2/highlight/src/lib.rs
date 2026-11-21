//! Shared language detection and syntax highlighting for every code surface
//! in Humata 2: diff views, file previews, code editors, Markdown fenced
//! blocks, and review views.
//!
//! Language detection checks, in order:
//! 1. exact file name (e.g. `BUILD.bazel`, `Dockerfile`, `Makefile`),
//! 2. file extension (case-insensitive, e.g. `.rs`, `.bzl`, `.toml`),
//! 3. content shebang (`#!/usr/bin/env python3`) or editor modeline
//!    (`vim: set ft=yaml:`, `-*- mode: starlark -*-`).

mod highlighter;
mod language;

pub use highlighter::{
    LineHighlights, highlight_code_block, highlight_line, highlight_lines, highlight_lines_cached,
    highlight_text, merge_overlay, shared_code_block_highlighter, theme,
};
pub use language::Language;
