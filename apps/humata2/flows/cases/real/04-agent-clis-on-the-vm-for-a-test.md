# A test puts pinned Copilot and Codex CLIs on the VM in their own homes, and takes them away after

Layer: real. Platforms: macOS host, Linux arm64 VM. This is test setup, not a user flow: it is what a later run (`real/06`, `real/08`, `real/09`, `real/12`) needs first.

## Starting state

- `h2-terminal-c1` runs and has been added as a machine (see `real/01-the-vm-becomes-a-machine.md`). `~/.humata2/bin` on the VM holds only Humata's tools.
- No `copilot` or `codex` is on the VM. The owner's real `~/.copilot`, `~/.codex` and `~/.claude` on this Mac and on the VM are never read or written.
- The VM has 1 CPU, 0.75 GiB RAM, a 4 GiB disk and no network except what Lima gives it. It has no host mounts, so files go over SSH.
- A temp folder on this Mac holds the run's fetched binaries (Bazel output) and notes.

## Test setup

- **Fetch the Copilot CLI with Bazel.** `apps/humata/copilot/copilot.bzl` is a `repository_rule` (`copilot_cli`) pinned by `_VERSION` and a SHA-256 per platform, including `linux_arm64`. `MODULE.bazel` (lines 370-390) already declares `@copilot_cli_linux_arm64`.
  - Its version is `1.0.110-1`. The Copilot adapter needs `1.0.112-0` or later (`daemon/conversation/src/copilot.rs` header, and the version read in `daemon/sessions/src/agents.rs` tests). The pin must move to `1.0.112-0` with new SHA-256s from the release's `SHA256SUMS.txt`, and the doc comment in `copilot.bzl` says to update `COPILOT_VERSION` in `apps/humata/daemon/agent/src/copilot.rs` too.
  - The binary is exported only to `//apps/humata/copilot:__pkg__` (`copilot.bzl` `_BUILD`), and that package's alias `:copilot` is visible only to `//apps/humata:__subpackages__`. A Humata 2 test target cannot depend on it. The narrowest fix is a new Humata 2 package that takes the file by a `genrule` or `filegroup` in `//apps/humata/copilot`, with `visibility` widened to that one package only.
- **Fetch the Codex CLI with Bazel.** There is no Codex rule (searched `MODULE.bazel` and `apps/`). Add a pinned `repository_rule` (or a module extension) that downloads one release asset for `linux_arm64` and, for the loopback tests on this Mac, `darwin_arm64`, with a SHA-256 per platform. The version is the one the terminal experiment used, `0.169.0` (`daemon/sessions/src/agents.rs` test `a_codex_or_copilot_kind_naming_no_version_args_records_what_its_cli_says`).
- **Copy the binaries into the VM.** From this Mac, with the Lima config (never the owner's `~/.ssh/config`):
  - `ssh -F "$HOME/.lima/h2-terminal-c1/ssh.config" lima-h2-terminal-c1 'mkdir -p /var/tmp/h2-real-clis/bin /var/tmp/h2-real-clis/home'`
  - `scp -F "$HOME/.lima/h2-terminal-c1/ssh.config" <copilot> <codex> lima-h2-terminal-c1:/var/tmp/h2-real-clis/bin/`
  - Verify with `sha256sum` on the VM against the pin.
- **Isolate the homes.** On the VM: `COPILOT_HOME=/var/tmp/h2-real-clis/home/.copilot`, `CODEX_HOME=/var/tmp/h2-real-clis/home/.codex`, and `CLAUDE_CONFIG_DIR` beside them. The daemon reads all three (`daemon/humatad/src/serve.rs:456-466`) and every agent it starts inherits them, so conversations are carried and found there. `HOME`, `XDG_*` and `COPILOT_CACHE_HOME` are set the same way, as `daemon/humatad/tests/continue_in.rs` `home_env` does.
- **Tell the daemon the kinds.** `codex` and `copilot` are not built in (`daemon/sessions/src/agents.rs`, `Agents::builtin` holds `claude` only; OpenCode is added separately). Write an `--agents` JSON on the VM and start the machine's daemon with `humatad2 serve --role machine --agents <file>`. Each entry has `name`, `program` (the absolute path in `/var/tmp/h2-real-clis/bin`), `start_args`, `resume_args`, `conversation_file`, `conversation_format` (`codex-rollout` or `copilot-session-state`), and optional `version_args`. The tested shape is in `daemon/humatad/tests/continue_in.rs` (line 191) and `codex_trust.rs` (line 31).
- **PATH.** SPEC TS-1 says sessions started on a machine have `~/.humata2/bin` on their PATH, but that folder holds only Humata's own four tools. I found no code that adds a folder to a session's PATH (searched `daemon/sessions/src`, `daemon/humatad/src`, `daemon/holder/src` for `PATH` and `.humata2/bin`). So use an absolute `program` in the `--agents` JSON, and do not rely on PATH. An agent's turn hook runs `humata turn` (`daemon/humatad/src/session_service/turns.rs:676`), which needs `humata` found, so check where `{humata}` resolves.
- **Login.** Either of these, never both mixed:
  - **Needs the owner's login.** Copilot device login and Codex `login`, run on the VM with the isolated `COPILOT_HOME`/`CODEX_HOME` set, so the token lands only in the run's temp home. Never copy a token from the owner's real home.
  - A deterministic loopback model server with isolated offline state (`terminal/provider/tests/copilot_resume.py`, `codex_resume.py`). It proves resume and fork without a login and does not prove a real model.
- **Why the isolated home is not optional (REALFLOW, from `briefs/real.md`).** The real Copilot CLI wrote `recentModelIds` into `~/.copilot/config.json` even for a "this session only" model pick. Without `COPILOT_HOME` pointing at the temp home, a run would change the owner's real config. Log in fresh inside the temp home; copying the owner's login in would be a decision for the owner and is not done here.
- **First-run prompts to expect (REALFLOW).** Codex shows a "Hooks need review" prompt (see `daemon/humatad/tests/codex_trust.rs`, Humata's own trust step). Copilot shows a folder-trust style prompt on first run in a new folder. Both appear once per isolated home, so a run's setup answers them or the test handles them.
- **Size check.** The Copilot tarball is about 200 MB (`copilot.bzl` doc), more once unpacked, and it runs on Node. Codex is a smaller binary (size not measured here). The VM has 4 GiB of disk, and Humata's add needs 2 GiB free (`daemon/machines/src/setup.rs:29`). Read `df -Pk /var/tmp` and `free -m` on the VM first. At 0.75 GiB RAM, one running Copilot plus `humatad2` and a holder is tight, and two CLIs at once may be killed by the kernel.
- **Cleanup.** After the run: stop the agents, remove `/var/tmp/h2-real-clis` on the VM, remove the throwaway agents JSON, and check `ls ~/.humata2` is the only Humata folder left. Never remove `/tmp/h2-terminal-holder` or touch other VM state.
- **Needs a free screen:** no. All of this is headless, over SSH and Bazel.

## Steps

1. **I do:** Run the Bazel fetch for the pinned Copilot linux_arm64 binary, then for Codex.
   **I see:** Two files in the output tree, and no download if the cache has them.
   **State and events:** Bazel checks each SHA-256. Nothing is run.
   **Status:** Code, no test for Copilot at the new pin (the existing pin `1.0.110-1` is in `copilot.bzl` and used by `apps/humata` only). Codex: Not built (no milestone; no `repository_rule` for it).

2. **I do:** Look at the target that exposes the Copilot file to a Humata 2 test.
   **I see:** A target visible to only the real-layer test package.
   **State and events:** `//apps/humata/copilot:copilot` is not visible to `//apps/humata2`.
   **Status:** Not built (no milestone). `apps/humata/copilot/BUILD.bazel` allows only `//apps/humata:__subpackages__`.

3. **I do:** Copy both files into `/var/tmp/h2-real-clis/bin` on the VM and check the SHA-256 there.
   **I see:** The `sha256sum` lines equal the pins. `file` says aarch64 ELF.
   **State and events:** Only that folder changes on the VM.
   **Status:** Code, no test. No test copies a CLI into the VM. The SSH config route is real (`terminal-ssh-demo.sh ssh-config`).

4. **I do:** Write the isolated homes and the `--agents` JSON, then restart the VM's machine daemon with `COPILOT_HOME`, `CODEX_HOME` and `CLAUDE_CONFIG_DIR` set.
   **I see:** Humata lists `codex` and `copilot` as session kinds on the VM.
   **State and events:** The daemon passes each home to its kind (`serve.rs:456-466`). A session's launch records the CLI's version (`version_args` or the format's default).
   **Status:** Works (real daemon) with fixture CLIs and a script program: `daemon/humatad/tests/continue_in.rs` (a codex and a copilot session list their turns; the JSON shape) and `daemon/sessions/src/agents.rs::a_codex_or_copilot_kind_naming_no_version_args_records_what_its_cli_says`. The real CLIs and the machine daemon on the VM are unproven. Restarting the VM daemon cleanly (the add's systemd or `setsid` start) is Code, no test over the real VM.

5. **I do:** Log in with the isolated home, or start the loopback server.
   **I see:** The CLI reports it is signed in, or reaches the loopback server with no network.
   **State and events:** The token file is under `/var/tmp/h2-real-clis/home`, and nowhere else.
   **Status:** Works (real shell) for the loopback route on this Mac only: `terminal/provider/BUILD.bazel` targets `copilot_resume_test`, `codex_resume_test`, `copilot_fork_test` (manual, macOS only, use the installed CLI). On the VM: Code, no test. The login route: Needs the owner's login.

6. **I do:** On the VM run `/var/tmp/h2-real-clis/bin/copilot --version` and `.../codex --version` with the isolated env.
   **I see:** `GitHub Copilot CLI 1.0.112-0.` (or later) and `codex-cli 0.169.0`.
   **State and events:** No file is written outside the isolated home.
   **Status:** Code, no test. Parsing these lines is Fakes only: `daemon/sessions/src/agents.rs::a_codex_or_copilot_kind_naming_no_version_args_records_what_its_cli_says`.

7. **I do:** Run `ls -la ~/.copilot ~/.codex ~/.claude` on the VM and on this Mac before and after the run.
   **I see:** Identical listings. Nothing new.
   **State and events:** The run's `ls` output is kept for comparison.
   **Status:** Code, no test. A real-layer assertion would diff these folders; none exists.

8. **I do:** Clean up as in Test setup.
   **I see:** `/var/tmp/h2-real-clis` is gone from the VM and `df` is back near the first reading.
   **State and events:** The VM keeps `~/.humata2` and `/tmp/h2-terminal-holder`.
   **Status:** Code, no test.

## Branches

### The SHA-256 does not match

- **When:** the download is changed or the pin is wrong.
- **I do:** Read the Bazel error.
- **I see:** Bazel stops the fetch and names the expected and actual hash. Nothing reaches the VM.
- **Expected outcome:** The failure says what was fetched and what to check. A bare "re-pin" is not enough: an unexpected hash may be a changed or tampered download, so confirm the release's `SHA256SUMS.txt` before changing the pin.
- **Status:** Code, no test. It is Bazel's own `download_and_extract` check (`copilot.bzl`). Humata does not add its own text.

### The VM is full

- **When:** the copy fails, or the add's Check sees less than 2 GiB free.
- **I do:** Run `df -Pk /var/tmp`, remove leftovers, copy again.
- **I see:** `scp` says `No space left on device`, or Humata's Check names the free and needed MiB.
- **Expected outcome:** The partial file is removed and the VM is left as it was. Humata's own message names the step and how to free space.
- **Status:** Fakes only for Humata's message: `daemon/machines/tests/setup.rs` (the `disk` case, `SetupError::DiskLow`). A partial `scp` is Code, no test.

### No network

- **When:** this Mac has no network during the fetch, or the VM cannot reach it.
- **I do:** Fetch before the run; the copy is local over SSH and needs no VM network.
- **I see:** Bazel says it could not download the file. The VM needs no network for the copy or the loopback model.
- **Expected outcome:** A cached fetch succeeds offline. A real-model run needs the VM's network, which Lima gives.
- **Status:** Code, no test.

### The CLI version is older than the adapter needs

- **When:** the Copilot binary is `1.0.110-1` (the current pin).
- **I do:** Start a Copilot session on the VM.
- **I see:** The adapter does not accept the layout.
- **Expected outcome:** The session is not read as a conversation, and the message names the version.
- **Status:** Code, no test. `daemon/conversation/src/copilot.rs` pins to 1.0.112 and later; no test starts the real 1.0.110-1.

### Kind not known to the daemon

- **When:** the daemon was started with no `--agents` file.
- **I do:** Start a Copilot session.
- **I see:** `copilot` is not offered. Only `claude` (and OpenCode) are.
- **Expected outcome:** The message says how to add the kind.
- **Status:** Likely bug (seen live, REALFLOW 2026-11-16): no built-in `codex` or `copilot` kinds. Not built as built-ins: ROAM batch 3, C4, X2 (`workflow/milestones/ROAM.md`).

### The run is interrupted

- **When:** the run dies before cleanup.
- **I do:** Run the cleanup command again.
- **I see:** The folder is removed. Nothing in the owner's homes changed.
- **Expected outcome:** Cleanup is safe to repeat.
- **Status:** Code, no test.

## Expected outcomes

- [ ] The pinned Copilot (1.0.112-0 or later) and Codex linux_arm64 binaries are fetched with a checked SHA-256 and copied to a test-only folder on the VM.
- [ ] Both CLIs run on the VM with `COPILOT_HOME` and `CODEX_HOME` in the run's temp folder, and their conversations are found there.
- [ ] The daemon on the VM offers `codex` and `copilot` through an `--agents` JSON.
- [ ] No file is changed in the owner's `~/.copilot`, `~/.codex`, `~/.claude`, `~/.humata2` or SSH keys.
- [ ] A login token, if used, exists only in the run's temp home.
- [ ] After cleanup the test folder is gone and the shared VM fixture is as it was.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Fetch pinned CLIs | Code, no test (Copilot at old pin); Not built (Codex rule) | `apps/humata/copilot/copilot.bzl`; no Codex rule |
| 2. Visibility for Humata 2 | Not built | `apps/humata/copilot/BUILD.bazel` |
| 3. Copy and check on the VM | Code, no test | Lima `ssh.config` |
| 4. Isolated homes and `--agents` JSON | Works (real daemon), fixtures | `daemon/humatad/tests/continue_in.rs`; `daemon/sessions/src/agents.rs::a_codex_or_copilot_kind_naming_no_version_args_records_what_its_cli_says` |
| 5. Login or loopback | Works (real shell) on this Mac for loopback; Code, no test on the VM | `terminal/provider/BUILD.bazel` `copilot_resume_test` |
| 6. CLI versions on the VM | Code, no test | `daemon/sessions/src/agents.rs` tests parse the line (Fakes only) |
| 7. Owner's homes unchanged | Code, no test | none |
| 8. Cleanup | Code, no test | none |
| Branch: SHA-256 mismatch | Code, no test | Bazel `download_and_extract` |
| Branch: VM full | Fakes only | `daemon/machines/tests/setup.rs` (`disk` case) |
| Branch: no network | Code, no test | none |
| Branch: CLI too old | Code, no test | `daemon/conversation/src/copilot.rs` |
| Branch: kind not known | Likely bug (seen live) | REALFLOW; `daemon/sessions/src/agents.rs` `Agents::builtin` |
| Branch: run interrupted | Code, no test | none |

## Gaps and likely bugs

- **Not built (no milestone):** a pinned Codex `repository_rule` or module extension.
- **Not built (no milestone):** a Humata 2 target that can see the Copilot binary. `//apps/humata/copilot:copilot` is visible to `//apps/humata` only, and its pin is `1.0.110-1`, older than the `1.0.112-0` the adapter needs.
- **Not built (ROAM batch 3, C4, X2):** built-in `codex` and `copilot` kinds. Today an `--agents` JSON is the only way.
- **Likely bug (seen live, REALFLOW 2026-11-16):** resume looks for the conversation under `--root` instead of the CLI's home, so a resumed Copilot or Codex session may not find its file. Setting `COPILOT_HOME` and `CODEX_HOME` on the daemon is the way to test it.
- **Likely bug (seen live, REALFLOW 2026-11-16):** `humata session read` is empty for Copilot, and a stopped session shows Idle, not Stopped.
- **Code, no test:** TS-1's claim that machine sessions have `~/.humata2/bin` on PATH. I found no code that sets it, and the folder holds no agent CLI anyway. An absolute `program` avoids it.
- **Risk to measure:** 4 GiB of disk against a 2 GiB free rule plus a ~200 MB (unpacked, larger) Copilot, and 0.75 GiB of RAM against Node plus `humatad2`.
