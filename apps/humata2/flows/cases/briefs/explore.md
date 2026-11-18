# Brief: explorers (recorded behaviour, step by step)

Writers (`briefs/verify.md`) say what *should* happen. Explorers record what
*does* happen: they step through the app's screens and the `humata` CLI with
the headless harnesses, and write down, per step, the screen, what it offers,
and what each action does. The step-2 tester replays each record as a check,
and the usability reviewer reads both.

Records go in `flows/cases/explored/<area>-<slug>.md`. Evidence (PNGs,
`.view.txt` copies, CLI transcripts, logs) goes in the session scratchpad
`verify/evidence/<area>/`.

## Record format

```
# <Area>: <what this record walks through>

Layer: explored · macOS · harness: <targets and test files>

## Setup
How the run was made: target, test filter, or the throwaway daemon's command
line (own --root and HOME), and anything seeded.

## Steps
### 1. <screen or command>
- **Screen:** what is on it (title, rows, buttons, empty states), from the
  .view.txt and the PNG.
- **Offered:** every action a person can take here (buttons, menus, keys).
- **Action:** what was done.
- **Result:** what happened: next screen, CLI output and exit code, events.
- **Evidence:** file under `verify/evidence/<area>/`, or the existing snapshot
  path.
- **Replay check:** one line a tester can re-run to confirm it (a target plus
  `--test_filter`, or a CLI command and its expected exit code).

## Not reachable headless
Screens or actions in this area that no harness reaches, and why.

## Surprises
Anything a person would find odd: dead buttons, unclear errors, status with
no fix, a missing undo. Facts only; the reviewer judges.
```

## Two phases

1. **Map from code (now, no Bazel).** Read the reducers (`features/*/src`),
   views (`ui/src`), the existing snapshots and their `.view.txt`
   (`desktop/tests/__snapshots__/{native,panels,shell,design}/`, viewable as
   images), the tests that make them (`desktop/tests/native/*.rs`,
   `desktop/tests/live_flows/*.rs`), and the CLI (`cli/src/args.rs`,
   `cli/src/commands/`). Write the record with every step you can fill from
   existing snapshots, and mark steps that need a run `PENDING RUN` with the
   exact command to make the evidence.
2. **Run (only when the coach says so, one explorer at a time).** Run the
   focused commands, fill the `PENDING RUN` steps, copy evidence.

## Rules

- Never git. Never cargo. Never edit repo files except your own records in
  `flows/cases/explored/`.
- Bazel only in phase 2, only when told, from the repo root:
  `export PATH=$PWD/.tools/bin:$PATH; bazel test --jobs=4 //apps/humata2/desktop:native_test --test_filter=<module>`
  and similar focused targets. Never `//apps/humata2/...`, never
  `design_test_record`, never `HUMATA2_SNAPSHOT=record`, never `clean` or
  `shutdown`. Test outputs (`.new.png` on a mismatch) are under
  `bazel-testlogs/<target>/test.outputs/`.
- Throwaway daemon: `bazel build --jobs=4 //apps/humata2/cli:humata //apps/humata2/daemon/humatad:humatad2`,
  then run `humatad2 serve` from `bazel-bin` with its own `--root` and a temp
  `HOME` under the scratchpad, and point the CLI at it with `HUMATA_SOCKET`
  (see `daemon/humatad/src/serve.rs` ServeArgs and
  `daemon/humatad/tests/common/mod.rs` for the flags tests use). Stop it when
  done. Never the owner's `~/.humata2`, `~/.claude`, `~/.codex`, `~/.copilot`.
- No windows on screen: headless renders only.
- Don't touch the design goldens, `prototype/`, or existing snapshots.
