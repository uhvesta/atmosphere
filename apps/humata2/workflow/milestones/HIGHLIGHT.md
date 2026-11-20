# HIGHLIGHT — code is highlighted in every language I read

**Owner, 2026-11-20:** "the syntax highlighting doesn't seem to work for Go or Rust or TOML or Java or Python or YAML or BUILD.bazel. or .bzl... you get the idea". This is a bug fix.

## Outcome
Every place Humata 2 shows code highlights it by its real language. That covers the diff and review view, the files preview, Markdown fenced blocks and agent output where it is code. At minimum: Rust, Go, Python, TOML, YAML, JSON, Java, Kotlin, C/C++, Swift, TypeScript/JavaScript, Shell, SQL, Terraform/HCL, Markdown, Dockerfile, Makefile, Protobuf, and Starlark (`BUILD`, `BUILD.bazel`, `MODULE.bazel`, `WORKSPACE`, `*.bzl`, `*.star`).

## Leads (main, 2026-11-20; unverified)
- `diff/src/model.rs:280-312`, `Language::from_path`:
  - a closed enum of 10 languages, with no Java, YAML, JSON, C or Starlark;
  - Starlark is mapped to Python, and `.bzl` falls to Plain.
- `diff/src/view.rs` never calls `.language()`. Nothing outside model.rs calls it either. The detected language may never reach the highlighter, which would explain why even Rust and Go show plain.
- **The highlighter itself:** gpui-kit is built with the `syntect` feature (MODULE.bazel, crate.annotation). Check:
  - whether the default syntax set is loaded;
  - whether the extension or token passed to it is right;
  - whether the theme gives the scopes colours in both light and dark.
- **Other surfaces** to find and route through one detector: the Markdown code blocks, the files and preview panes, and the review chat.

## Exit
- **One shared language detector** (a single-job crate per D74): by file name first, then by extension, then by shebang or modeline. Every surface uses it.
- **Failing-first tests:**
  - a table test of file name → language for every language above;
  - a test per surface that asserts highlighted spans (more than one colour) for one sample per language, headless.
- **Design goldens:** one new frame per surface showing Rust, Go, YAML and Starlark highlighted. Fable judges them.
- **Performance:** highlighting stays off the UI thread, and large files stay within the diff bench gate (`diff/benches/gate.rs`).

## Ships
0.10.2, with NEWCHCLOSE and MACHINEARCHIVE.

## Out of scope
Semantic highlighting, LSP, editing.
