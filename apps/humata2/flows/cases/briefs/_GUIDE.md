# Writer guide for Humata 2 use cases

Read this guide, then the brief for each case assigned to you. Write one
Markdown file per case at the path the brief names.

## What a case is

A case is a test case written in prose. It describes a real flow, and it must
be clear enough that someone can turn it into code later from the events it
names. It is not code and it is not a spec.

Each case has these parts, in this order:

1. `# <Title>`. The title is a sentence that names the outcome, for example
   "A typo fixed with ⌃W runs the right command".
2. One line naming the layer (micro, macro or agents) and the platforms it
   covers (macOS, Linux, web).
3. `## Starting state`. Bullets: what is running, which machines exist, what
   is open and focused, and what the user or agent is holding.
4. `## Steps`. Numbered. Each step has three parts:
   - **I do:** what the person or agent does, down to the key or command.
   - **I see:** what appears on the screen or comes back from the command.
   - **State and events:** what the system should record, where that is
     useful. Examples: "the session shows Stopped", "an Inbox request
     appears with Allow, Deny and Always", "the terminal gets the bytes `^U`",
     "the CLI exits 12".
   - **Status:** one of the tags below.
5. `## Branches`. One `###` heading for each branch: an error, offline, a
   wrong choice, a permission ask, a concurrent edit, or a restart or reboot
   in the middle of a step. Each branch says when it happens, what I do, what I
   see, and the expected outcome, and has its own **Status**.
6. `## Expected outcomes`. A checklist (`- [ ]`) of the results a test would
   assert on. These are behaviour, not wording.
7. `## Status summary`. A table with the columns Step or branch, Status, and
   Evidence.
8. `## Gaps and likely bugs`. Bullets. Separate missing features ("Not
   built") from likely bugs, which are things the code does that look wrong.

## Status tags

Tag every step and every branch. Check the code and the tests, not the spec
alone. The spec describes what is planned, so it does not show what exists.

- **Works (real shell)** or **Works (real daemon)**: a test proves it against
  a real shell, a real daemon, or the real SSH VM. Name the test as
  `path/to/file.rs::test_name`.
  - The terminal lab tests in `terminal/lab/tests/` use a real zsh, a real
    Neovim, and a real OpenSSH VM.
  - `desktop/tests/live_flows*`, `live_inbox.rs` and `live_boards.rs` run a
    real `humatad2` with a fake agent.
  - `daemon/humatad/tests/*` run a real daemon in a temp folder, with fake
    agents and fake boxes.
- **Fakes only**: a test proves it, but only against fakes. Name the test and
  say what is still unproven live. Most of `features/*/tests`,
  `desktop/tests/native/*` and `clients/tests` are fakes. Before you call
  something Works, check what the test builds on.
- **Code, no test**: the code exists but no test covers it. Name the file.
- **Not built**: name the milestone or requirement ID, or write "no
  milestone".
  - The daemon answers `status::not_built("...")` for RPCs that are not built.
    Search with `grep -rn 'not_built(' apps/humata2/daemon/humatad/src`.
  - MCP tools that `daemon/humatad/src/mcp.rs` (`impl Daemon for
    GrpcDaemon`, `fn call`) does not match fall into `other =>
    Err(not_built(other))`.
  - To find a milestone, look in `apps/humata2/workflow/DAG.md` and
    `apps/humata2/workflow/milestones/*.md`.
  - To find a requirement, run `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md`.
- **Likely bug**: reading the code shows something that looks wrong, for
  example a key that does nothing. Name the file and line, and say how to
  confirm it.

If you cannot find evidence either way, use "Code, no test" or "Not built",
and say what you searched for. Never guess "Works".

## Where to look

Everything lives under `apps/humata2/`.

- **App UI:**
  - `ui/src/` holds the views and the key bindings. The bindings are in
    `ui/src/keys.rs`, and the menus in `desktop/src/main.rs`.
  - `features/*/src` holds the reducers.
- **Terminal:**
  - `terminal/vt` holds the emulator.
  - `terminal/view/src/keys.rs` decides what each key means.
  - `terminal/pane` holds the find bar, links and paste confirmation.
  - `terminal/client` holds the replica the app embeds.
  - `terminal/lab` holds the real-shell tests.
- **Daemon:**
  - `daemon/humatad/src` holds the services.
  - `daemon/holder` keeps the PTYs alive.
  - `daemon/access` holds the rules.
  - `daemon/mcp` holds the MCP tools.
- **CLI:**
  - `cli/src/args.rs` and `cli/src/commands/` are what exists.
  - `api/CLI.md` is the plan for the CLI.
  - CLI exit codes: 0 ok, 2 usage, 3 not found, 4 denied, 5 user only or
    presence required, 6 conflict, 7 preconditions, 8 offline, 9 external
    tool, 10 disk full or rate limited, 11 checksum, 12 pending, 20 journaled
    and strict, 70 internal, 130 interrupted.
- **Specs:**
  - `SPEC.md`: the UX flows are §26.
  - `DECISIONS.md`: D-numbers, and the newest decision wins.
  - `OVERVIEW.md` lists known gaps. It can be out of date, so check it
    against the code.
- **Commands for reading:** `grep -rn`, `grep -o 'fn [a-z_0-9]*('
  <file>` to list test names, `sed -n`, and `ls`.

## Rules

- Write only the files your brief names. Do not edit anything else.
- Never run bazel, cargo or git. Only read files and search them.
- Do not put in personal details: no usernames, no home paths, and no real
  repo URLs or org names. Use `acme/api`, `box-1`, `this Mac` and similar.
- Never name products that inspired the design. Describe behaviour in
  Humata's own words.
- Write short sentences in plain English. Use `⌘`, `⌥`, `⌃` and `⇧` for keys
  on macOS and `Ctrl`, `Alt` and `Shift` elsewhere.
- Aim for 80–200 lines per case. Thorough beats short, but do not pad.
- When you finish, reply with: the files you wrote; for each case, the count
  of steps and branches by status tag; and every Not built and Likely bug item,
  one line each.

## Findings so far (added by the coach during wave 1)

- **Pane events the app drops.** The app's terminal glue, `ui/src/terminals.rs` (around lines 178–212), handles only these events: Focused, SplitRequested, ClipboardWrite, Failed and InputUncertain. Every other `PaneEvent` falls into `_ => {}`, so the app drops these:
  - OpenLink, so ⌘-click does nothing;
  - PasteImage and PasteFiles, so image and file paste do nothing;
  - Bell;
  - TitleChanged;
  - PwdChanged;
  - ClipboardReadRequested.

  `terminal/client/src/lib.rs` handles some of these, but it is the standalone lab client, not the app.
- **Notice frames are dropped.** `clients/terminal_link/src/lib.rs:191` drops every Notice frame from the daemon.
- **Tests without a shell.** Engine-only tests, such as `terminal/vt/tests/*` and the unit tests in `terminal/view/src/keys.rs`, count as **Fakes only**. They run no shell.
- **Manual lab tests.** The `terminal/lab` interactive and SSH tests are manual targets. They count as Works (real shell) because they prove the behaviour when they run. Note that they are manual.
