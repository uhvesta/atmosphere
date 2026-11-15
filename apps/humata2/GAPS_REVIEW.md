# Known-gaps review log

What was done against `OVERVIEW.md › Known gaps (2026-11-15)`, in order, so a reviewer can check each change. Each entry names its commit, the gap it closes, how it was verified, and what to look at. Add an entry for every commit that works on a gap or a milestone item.

## How to review

- `git show <commit>` for each entry below; read the test it names first.
- Re-run the named Bazel targets (never cargo): `export PATH=$PWD/.tools/bin:$PATH; bazel test <target>`.
- Snapshot images live beside their tests in `__snapshots__/`; open them.

## Entries

### e4ec2d5d: the known-gaps list
- Adds the Known gaps section to OVERVIEW.md. Review that nothing reported by the agents is missing.

### a7c6cf3c: desktop panes and snapshot fixes (Desktop UI)
- `ui/src/panes.rs` draws `state.panes.tree()` as rows and columns with a header per pane (title, close button; a click focuses the pane). `ui/src/shell.rs` handles ⌘\, ⇧⌘\, ⌘W, ⌘⌥ arrows and ⌘Z for layout.
- Home's column is centred, and the Inbox badge sits off the rail icon.
- Verified: `bazel test //apps/humata2/desktop:shell_test` (new test `splitting_draws_two_panes_and_closing_one_goes_back`, re-recorded `home.png`).
- Look at: a single pane draws without chrome (no header), on purpose. Divider dragging and pane drag-and-drop are not built.

### 1818f806: markdown toolbar and Source toggle (Markdown)
- `markdown/src/view.rs`: the toggle keeps keyboard focus; the toolbar caches its selection and acts on mouse down.
- Verified: in `headless:flows`, `the_keyboard_switches_to_the_source_and_back` passes. Two flows still fail, as recorded in OVERVIEW.
- Look at: whether acting on press, not click, is acceptable UX for the selection toolbar.

### 639904a6: Copy as Markdown keeps block markup (Markdown)
- `markdown/src/view.rs` `with_block_markup`: when a selection starts at a block's first visible character, or ends at its last, the source range grows over the markup only (`# `, `> `, list markers, closing `**`).
- `testing/flow`: `Target::at_start()` aims a click or drag just inside a control's leading edge. The copy flow uses it for the heading.
- Verified: `bazel test //apps/humata2/markdown/headless:flows` (copy flow passes; 6/7), plus `markdown:markdown_test`, `headless:rendering` and `headless:snapshots`.
- Look at: the markup character set in `only_markup` is a heuristic.
