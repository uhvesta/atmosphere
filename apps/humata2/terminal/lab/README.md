# Terminal Lab (minion C1)

An isolated macOS app for embedding real local and SSH terminals. It uses the existing PTY holder and Ghostty terminal engine; the main Humata app is not rewired by this experiment.

## Open-source stack and design

The terminal engine is **libghostty-vt**, built from the repo’s pinned Ghostty source through Bazel. Its parser, terminal state, key/mouse encoding and VT modes are reused. The existing `terminal/view` GPUI renderer and `TerminalPane` supply the grid, selection, find, copy/paste and terminal events. The existing Rustix/Tokio PTY holder owns each real process, and OpenSSH provides the remote transport. WezTerm and Alacritty are not dependencies of this experiment. New code is the lifecycle/persistence adapter and small host app, rather than another terminal emulator.

The host follows the main app’s D76 colors, traffic lights, 236px sidebar, flat terminal tabs, compact bordered controls, blue primary actions and Menlo terminal typography. Light/Dark switches update both the chrome and the terminal’s default colors without replacing its process. The sidebar hides below900px and returns when the window grows.

The reusable component has ordinary GPUI and **GPUI Fast** targets: `embed` and `embed_fast`. The lab builds as `terminal_lab` and `terminal_lab_fast`; `lib` and `lib_fast` use the corresponding engine consistently. This matters because the existing GPUI prototype uses Fast. Its current `ui-prototype/src/terminal.rs::open` still returns a demonstration-grid view: adopting the real component there also requires routing its session/pane lifecycle and unsent-feedback model to the real attachment. This experiment leaves that simulated prototype intact; it does not silently make staged screens execute commands.

## Run

From the repository root:

```sh
export PATH=$PWD/.tools/bin:$PATH
bazel run //apps/humata2/terminal/lab:terminal_lab -- --data /tmp/my-terminal-lab
```

Use `terminal_lab_fast` instead of `terminal_lab` in the command to try the prototype’s GPUI Fast engine. Use **+ Local** to start shells. Drag tabs to reorder them or move between the two groups; the Move button provides the same operation. Moving retains the same view and process. **Detach** closes the attachment while the process runs on. **Resume** reconnects and catches up. **Stop** explicitly ends the process. Quitting the app leaves holders running. Reopen with the same data folder to restore the terminal list, tab groups, selected tabs and saved terminal display.

**Send text** pastes into the last focused terminal using its terminal modes. It does not implicitly press Enter. Multiline text keeps TerminalPane's confirmation when bracketed paste is unavailable. Normal keyboard input goes only to the focused terminal; the text field and find bar have their own focus. TerminalPane supplies mouse reporting, selection/copy, scrolling, find, colors, cursor/modes, alternate screen, bracketed paste, Unicode and terminal query replies.

The default data directory is `~/.humata-terminal-lab`. Use a short dedicated path: Unix socket paths have platform length limits. Existing directories must already be private (0700); the component refuses shared directories instead of chmodding them. `--holder /absolute/path/to/humata2_holder` overrides the bundled Bazel runfile.

## Reusable components

- `//apps/humata2/terminal/session:session`: UI-independent local/SSH sessions, input acknowledgements, resize, persistent descriptors/layout and a durable output/size journal. It uses the existing holder protocol and never imports app reducers or GPUI.
- `//apps/humata2/terminal/embed:embed`: reusable `EmbeddedTerminal` GPUI entity. Construct it from a session descriptor and attachment, host it in any layout, drain its bounded event queue with `poll`, and call `send_text` for programmatic paste. `set_appearance` takes host-supplied colors/font settings so the view follows the embedding app’s design. `pane()` exposes existing TerminalPane events/APIs. `EmbeddedEvent::Focused` lets a host track the last focused terminal. Detaching/dropping the component closes only its I/O tasks.
- `//apps/humata2/terminal/lab:terminal_lab`: this small host app. It chooses a two-group tab layout; the terminal view itself does not own tabs/groups or depend on this app. Bazel visibility currently names only actual consumers; add a narrow consumer when integrating elsewhere.

`SessionStore::create` starts a new program from an arbitrary executable/arguments/cwd descriptor. `attach` reconnects to a known ID; `Reader` receives output/acknowledgements, and `Writer` serializes raw input/resize/stop. Give each Reader one dedicated task: its framed stream is not cancel-safe. Never automatically retry input whose acknowledgement was lost. `Writer::stop` requests shutdown; retain its Reader/tunnel until EOF confirms shutdown. `SessionStore::stop` does this waiting for a detached session. Dropping an SSH tunnel immediately after a write can discard bytes still in that transport.

## Persistence and resume semantics

Each terminal runs under a separate holder process. Its shell's environment, current directory, foreground program and PTY survive UI exit, client crash and SSH transport loss **while that holder/host stays alive**. The holder retains a 16 MiB catch-up window and its existing disk scrollback log.

The local workspace keeps private files:

- `workspace.json`: versioned descriptors, a random workspace namespace, terminal IDs, group order and selected tabs. Updates sync a temporary file, rename it, then sync the directory.
- `sN.journal`: checksummed frames containing output byte offsets and resize operations. Frames sync before output is exposed to the UI. Replay reconstructs terminal cells, cursor, scrollback and VT modes; it suppresses terminal replies and clipboard side effects during replay. An incomplete final frame is repaired; a checksum failure is reported without overwriting the file.
- Local sessions additionally have holder sockets/logs/scrollback beside the manifest. SSH sessions use a per-workspace namespace beneath the configured remote root, avoiding collisions between different local workspaces.

The workspace lock rejects a second owner. A journal lock rejects two frontend attachments to one saved terminal. Separate terminals remain independent. A trim marker from the holder becomes a visible replay-gap notice and resets the parser; the component does not silently present truncated output as complete state.

**Resume is reattachment, not process resurrection.** If a holder is killed or its machine reboots, the app can restore its locally saved display, but cannot recover arbitrary process memory. It shows a detached/error state rather than silently replacing the process. Starting an agent with its own conversation-resume arguments is a host application decision. No new process is started by Resume.

## SSH experiment with a small Lima VM

This session verified a separate `h2-terminal-c1` Debian aarch64 VM with 1 CPU, 0.75 GiB RAM and a 4 GiB disk. Existing VMs were not changed. Install a holder matching the client build on the remote machine. The tested commands are:

```sh
limactl start --name=h2-terminal-c1 --cpus=1 --memory=0.75 --disk=4 \
  --vm-type=vz --mount-none --containerd=none --tty=false template:debian
bazel build --config=linux_arm64 \
  --extra_execution_platforms=@platforms//host,//bazel/platforms:linux_arm64 \
  //apps/humata2/daemon/holder:humata2_holder
limactl copy bazel-bin/apps/humata2/daemon/holder/humata2_holder h2-terminal-c1:/tmp/h2-terminal-holder
limactl shell h2-terminal-c1 chmod 700 /tmp/h2-terminal-holder
```

Use Lima's generated SSH config; it supplies the SSH key, host alias and host-key policy. The component does not disable authentication or host-key checking. It uses `BatchMode` and independent SSH Unix-socket tunnels, with keepalives and bounded startup. No TCP listener is exposed. The matching remote holder remains alive when its SSH attachment closes.

```sh
bazel run //apps/humata2/terminal/lab:terminal_lab -- \
  --data /tmp/my-ssh-terminal-lab \
  --ssh-config "$HOME/.lima/h2-terminal-c1/ssh.config" \
  --ssh-host lima-h2-terminal-c1 \
  --remote-holder /tmp/h2-terminal-holder \
  --remote-root /var/tmp/terminal-lab
```

Then use **+ SSH**. Existing saved SSH sessions retain their endpoint details, so reopening the same workspace reconnects them. `/tmp/h2-terminal-holder` is a test installation; use a durable executable location when testing machine reboots. Saved output cannot make a dead holder's old process run again.

## Verification

See [ACCEPTANCE.md](ACCEPTANCE.md) for the component boundary, stress matrix, observed performance and remaining conditions.

All builds, tests and formatting use Bazel. Tests explicitly use `/bin/sh` rather than loading the user’s interactive shell startup files. UI tests run headlessly, with no on-screen window.

```sh
bazel build //apps/humata2/terminal/lab:terminal_lab //apps/humata2/terminal/lab:terminal_lab_fast
bazel test //apps/humata2/terminal/session:session_test \
  //apps/humata2/terminal/lab:app_test //apps/humata2/terminal/lab:app_fast_test \
  //apps/humata2/terminal/pane:pane_test \
  //apps/humata2/daemon/holder:terminal_test --test_output=errors

bazel test //apps/humata2/terminal/session:ssh_test \
  --test_env=TERMINAL_LAB_SSH_CONFIG="$HOME/.lima/h2-terminal-c1/ssh.config" \
  --test_output=errors
bazel test //apps/humata2/terminal/lab:app_ssh_test //apps/humata2/terminal/lab:app_fast_ssh_test --strategy=TestRunner=local \
  --test_env=TERMINAL_LAB_SSH_CONFIG="$HOME/.lima/h2-terminal-c1/ssh.config" \
  --test_output=errors
```

The SSH tests are explicit/manual and need that test VM and installed holder. Keep the GUI build/link actions sandboxed; run only the SSH test action locally. Making the whole GUI target `no-sandbox` lets the linker find an undeclared Ghostty dylib beside its static archive, yielding an unusable executable.

The local test kills an actual client process, reopens its workspace and verifies the same shell PID/environment and synced history. SSH tests drop the tunnel and workspace, verify output produced during the disconnection, reattach to the same remote PID, resize the remote PTY and prove a second shell is isolated. Headless app flows assert every expected output row stays painted across eight idle captures, create another shell through the actual Local/SSH button, and verify actual rendering, cross-group drag, tab reorder, paste/focus isolation, detach/resume, alternate-screen replay and half/full resize using real PTYs, locally and over SSH.

Capture app images with `bazel run //apps/humata2/terminal/lab:app_test_record`; use `app_ssh_test_record` with the SSH test environment for remote captures. Use `app_fast_test_record` / `app_fast_ssh_test_record` for the Fast engine. Images live under `tests/__snapshots__/app`, with separate engine and light/dark names.

## Limits before product integration

This is an experiment. The UI-independent session/view boundaries are ready to try in other hosts; the main app still uses its existing fake terminal transport. The journal currently grows on disk and replays in memory, so retention/compaction, indexed checkpoints and long-running throughput need work before shipping. The component supports one journaled frontend attachment per terminal; external holder clients can still affect PTY size. Replay requires a complete output/size history. Rendering and background I/O have separate queues, so pixel-identical replay while output races a resize is not yet guaranteed; a renderer checkpoint or a unified presentation journal is a follow-up before making that promise.

File/image upload and granting programmatic clipboard reads are not implemented in the lab; it reports those requests rather than silently pretending they succeeded. Explicit text paste/copy and program-requested clipboard writes use the existing pane path. Window geometry, selection/find UI, app text drafts and scroll position are not persisted. The lab polls events every16 ms; a production host should integrate its wake mechanism. Automatic remote provisioning, holder upgrades, boot recovery/agent conversation resume and integration with real Humata daemon APIs remain separate work.

Browser feasibility and the transport split are recorded in [BROWSER.md](BROWSER.md); no browser implementation is claimed yet.

## Terminal interaction and engine boundary

The component uses the repository's existing **libghostty-vt** state/parser/input engine and **TerminalPane/GPUI renderer**; it does not embed Ghostty's native Metal surface. There is no new terminal escape parser. Alacritty was evaluated against the required VT colors, scrollback, selection, modes and resize APIs. Its core supports those, but is not a ready-made GPUI renderer: [Alacritty Term API](https://docs.rs/alacritty_terminal/latest/alacritty_terminal/term/struct.Term.html), [Zed terminal dependency boundary](https://github.com/zed-industries/zed/blob/main/crates/terminal/Cargo.toml), [Zed's separate GPUI terminal element](https://github.com/zed-industries/zed/blob/main/crates/terminal_view/src/terminal_element.rs). No Zed implementation was copied. Switching state engines alone would not supply rendering, process lifetime or SSH persistence; the existing engine remains behind the generic Grid boundary.

ANSI, indexed and truecolor output, styled Unicode text, alternate-screen TUIs, selection/copy, scrollback, find, mouse reporting and focused input use the existing terminal pane. Shift-drag selects even when a TUI owns the mouse. OSC 8 metadata and plain browser/mail URLs are clickable with Cmd-click (Ctrl-click off macOS); hover previews the destination. The host allows http/https/mailto and reports other schemes instead of executing them. Ordinary clicks remain terminal selection or application mouse input.

One terminal fills the workspace. **Split** or Cmd+\ creates another real shell in the other group; tabs drag between groups. Cmd+T creates a local shell in the active group. Cmd+plus/minus/0 zooms or resets terminal text. **Send text** pastes into the active live terminal and returns focus so Enter reaches that terminal. Sending to a detached/exited process reports the problem and retains the draft. Light mode has a separate ANSI palette; explicit program RGB output remains exact.

## Real Neovim acceptance

The manual target launches an installed Neovim in the same persistent PTY path, with `--clean -n -i NONE` to avoid the user's config, swap and shada. It checks real RGB syntax colors, mouse cursor movement, typed edits, font/window resize, detach/reattach with an unsaved buffer, and writing that buffer to disk. It is intentionally an explicit system-application acceptance test, not a newly pinned Neovim dependency. Keep its build/link sandboxed; only run the acceptance action locally.

```sh
bazel run //apps/humata2/terminal/lab:neovim_test_record \
  --test_env=TERMINAL_LAB_NVIM=/opt/homebrew/bin/nvim -- --nocapture
bazel test //apps/humata2/terminal/lab:neovim_test \
  //apps/humata2/terminal/lab:neovim_fast_test --strategy=TestRunner=local \
  --test_env=TERMINAL_LAB_NVIM=/opt/homebrew/bin/nvim --test_output=errors
```

Use `neovim_fast_test_record` for the Fast capture. Captures are under `tests/__snapshots__/neovim/` and are rendered from Neovim's actual PTY stream.

## Owner-shell and SSH acceptance with large scrollback

`interactive_test` and `interactive_fast_test` are manual system acceptance targets. They launch Neovim beside an actual interactive zsh, run `ls`, `cd`, and read-only `git status`, verify the af-magic prompt, paste via Cmd+V, select/copy output via Cmd+C, use Up/Down shell history, drag the terminal across groups, reconnect its existing process, generate **12,000 real output lines**, navigate Page Up/Top/Bottom, and verify idle rendering reuses cached rows. Use `--test_arg=--nocapture` with `bazel test` to retain their observed output delivery, replay and scroll timing; these are headless acceptance measurements, not screen-present latency or a universal performance guarantee.

```sh
bazel run //apps/humata2/terminal/lab:interactive_test_record \
  --test_env=TERMINAL_LAB_NVIM=/opt/homebrew/bin/nvim \
  --test_env=TERMINAL_LAB_REPO="$PWD" \
  --test_env=TERMINAL_LAB_ZSH_HOME="$HOME" -- --nocapture
```

The local target intentionally loads the owner's zsh configuration, disables Oh My Zsh automatic updates, and redirects its compdump/cache/history into the test's temporary folder. Its images contain changing real prompt/Git output, so they are captures rather than portable fixed pixel goldens. It does not edit tracked checkout files.

The same target with `--test_env=TERMINAL_LAB_SSH_CONFIG=...` uses the dedicated VM fixture at `/var/tmp/h2-terminal-acceptance`, remote zsh/Neovim/Git, and an isolated `ZDOTDIR=/tmp/h2-terminal-zdot`. That fixture contains only the public Oh My Zsh framework, git plugin and af-magic theme, with updates disabled; no local dotfiles, credentials, private plugins or history are copied to the VM. Commands, local clipboard shortcuts, movement, reconnect and 12k history use the real SSH transport. The VM setup is deliberately explicit/manual, rather than silently provisioning user hosts.

Neovim also negotiates DEC mode 2048 in-band resize notifications. The pane now delivers the engine's resize replies immediately, without waiting for further process output. This avoids a deadlock where a TUI waits for a resize report while the frontend waits for a redraw. The pane regression test exercises that case without generating additional output.
