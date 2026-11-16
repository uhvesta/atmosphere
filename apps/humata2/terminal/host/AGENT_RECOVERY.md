# Agent conversation recovery

A surviving terminal process is reattached, not restarted. The holder owns the
PTY independently of the viewer, local Host and SSH tunnel. A killed agent is a
different case: its process memory is gone, but its provider may resume a saved
conversation from its own durable state. These operations have separate tests
and separate APIs.

## Explicit opt-in

Ordinary `Host::start` never starts a replacement agent. Native embeddings opt
in with `Host::start_with_agent_resume(config, store, profiles, resume)`:

```rust,ignore
use std::{collections::BTreeMap, sync::Arc};
use terminal_host::{AgentResumeConfig, provider::{ResumePolicy, ResumeRegistry, CODEX_RESUME_V1}};

let resume = AgentResumeConfig {
    registry: Arc::new(ResumeRegistry::default()),
    profiles: BTreeMap::from([("codex".into(), ResumePolicy::Enabled {
        adapter: CODEX_RESUME_V1.into(),
        provider: "codex".into(),
        args: vec!["--no-alt-screen".into()],
    })]),
};
```

The map keys are trusted launch profile names. Each profile still supplies its
program and endpoint. `args` contains explicitly configured **resume-only** CLI
options, never the original launch prompt or a previous session selector. The
planner constructs the full argument vector with the exact saved conversation
ID. It does not use a shell, latest-session lookup, a picker, a name or a UUID
prefix. Canonical full UUIDs are required by the three builtins; a registered
custom adapter may support another provider's explicit ID format.

The standalone Host also accepts `--agent-resume FILE`, a bounded trusted JSON
map with the same profile names:

```json
{
  "codex": {
    "mode": "enabled",
    "adapter": "codex-cli-resume-v1",
    "provider": "codex",
    "args": ["--no-alt-screen"]
  },
  "claude": {
    "mode": "enabled",
    "adapter": "claude-cli-resume-v1",
    "provider": "claude",
    "args": []
  },
  "copilot": {
    "mode": "enabled",
    "adapter": "copilot-cli-resume-v1",
    "provider": "copilot",
    "args": ["--no-auto-update"]
  }
}
```

Configure every named profile before starting the Host. No policy or an explicit
`{"mode":"disabled"}` keeps replacement disabled. Unknown policy fields cannot
select a program or endpoint. The JSON file is limited to 1 MiB; individual fields
and argument vectors have smaller typed limits. Custom adapters are registered
through the native API, not loaded as executable code from JSON.

## Session identity and launch ownership

The launcher supplies `Profile.recovery` for an already-known conversation, or
binds structured provider events with `bind_provider(...).ingest(...)` so
`SessionStarted` records the ID/cwd. The authenticated controller can explicitly
set a recovery hint through the existing wire API. Ordinary terminal output is
never scanned for an ID. Starting a CLI without a known ID or structured event
does not make its conversation automatically discoverable.

On a positively reported **nonzero child exit**, an enabled profile plans one
recovery attempt. The new process uses the same machine and saved cwd, a fresh
holder/terminal UUID, and the exact provider conversation ID. The old terminal
and display remain inspectable. The new record's `terminal.resume.source`
attribute links back to the old UUID; consuming apps decide whether to replace a
tab, open a new one or show a notice. New viewport state is not copied from the
old VT, and pending input, draft text and the original prompt are not replayed.

The reservation, provider hint and source/profile attributes are written
atomically **before** spawning. Retrying the same source finds its durable
reservation rather than launching twice, including after Host restart. An
incomplete/unavailable or conflicting reservation returns an explicit error;
it is not reported as a live resumed process. A resumed child does not create an
automatic crash loop if it also fails. Native callers may deliberately recover
that later child as a separate action.

Explicit native/viewer Stop saves a private `resume_blocked` intent before
sending the stop. This prevents its nonzero exit code from restarting the agent
after Host restart. Normal exit 0 is not an automatic recovery trigger. A Host
shutdown drains an in-flight launch's completion before releasing its workspace
lock; it does not assume canceling a blocking spawn can undo a started process.

`observe_agent_resume()` exposes bounded native events, without installing OS
notifications. `Launched(AgentResumeLaunch)` means a process was launched and
attached, **not** that a provider accepted its conversation or completed a turn.
Verify that through structured provider events or the provider itself. Failed
launches retain sourced typed errors; observers receive an explicit lag count
when they miss events. Existing reservations are flagged `existing=true` only
when their recorded identity matches and their process is live.

## Forked conversations

An agent conversation fork is distinct from `TerminalHandle::spawn_from`, which
opens an independent fresh terminal at the same machine/cwd. A provider fork
creates another conversation ID and retains its parent in the provider's own
state. Recovery must select the new foreground ID, never repeat the fork flag.

When the launcher observes the actual foreground fork, use:

```rust,ignore
binding = binding.activate_fork(parent_session_id, fork_session_started, fork_state_paths).await?;
```

The operation compares the expected parent with the current recorded identity,
persists the new structured ID and explicitly supplied child state paths, then
returns a new binding. Parent-specific file paths are not inherited by guessing.
The old binding is revoked. The foreground fork is pinned, so delayed background-parent events,
including SessionStarted, cannot change recovery back to the parent. That pin
survives Host restart; a settings/checkpoint race cannot override the
workspace's authoritative saved conversation ID. If the workspace identity is
saved but companion settings fail, `RecoverySettingsCommitted` reports that
partial commit: the new ID remains selected in memory and on disk, the old
binding is revoked, and callers must repair the folder/disk and bind again.
They must not repeat the parent fork operation. A later foreground fork uses
the new binding and its current ID as the expected parent.

The terminal component does not infer forks from prompts or slash-command text.
An integration must forward the provider's actual structured fork/session
activation. Background conversations keep their provider state, but resuming one
foreground branch does not promise to resurrect every concurrent background
job after the entire CLI process is killed.

Real Codex, Claude Code and Copilot fixtures create forks through the installed
CLI, obtain distinct child IDs, commit fork-only work, kill a pending child
turn, and resume that exact child without another fork operation. They verify
inherited parent history and committed fork history, then explicitly verify
that the parent contains no child-only conversation turns. Provider branch
metadata may legitimately change in both files.

Codex uses `exec fork PARENT`; Claude uses `--resume PARENT --fork-session`.
Copilot uses its actual interactive `/fork branch-name`, which changes the
foreground while the parent can continue in the background. Its fixture then
uses exact-ID resume to commit a child turn. Codex retains its unfinished child
turn; Claude and Copilot roll it back. The component does not replay any of
these interrupted inputs. These are conversation-fork tests, not a promise to
restore concurrent background processes.

## SSH, holder and machine death

Failed SSH connection attempts are not evidence that an agent died. The Host
first retries the existing holder; it never starts another agent merely because
a socket or remote machine is unreachable. The real SSH test matrix covers
SIGKILL of the owned tunnel, local Host, remote holder and remote child.

When a trusted orchestrator has independent evidence that the old process
cannot run (for example, a completed machine reboot), it may call
`resume_agent_after_confirmed_loss(uuid)`. This native-only operation uses the
same opt-in policy and durable reservation; a browser viewer cannot make that
attestation. It still refuses to replace a terminal currently reported live or
stopping. The same endpoint must remain configured. If the provider's state
files were lost with the machine, a session ID alone cannot recreate them.

A dead holder without positive child-exit evidence remains Unavailable, with
its last checkpoint read-only. A disk checkpoint is display/parser state, not
process memory.

A separate disposable Lima/VZ guest test force-stops the whole VM, verifies a
changed kernel boot ID, and confirms synced conversation/history and installed
executable hashes survive. The local Host remains Unavailable without launching
another agent. Only after independent new-boot evidence does the native recovery
API create a fresh terminal UUID/process at the same cwd with the
exact saved conversation ID. The strict remote agent is a disk-backed fixture;
real vendor conversation/fork tests run separately on macOS. These results do
not certify storage destruction or every vendor's credentials/trust setup.

Durable installation matters too: the first power-cut probe left unsynced copied
executables as zero-byte files, while synced conversation state survived. The
fixture now syncs its executable files and parent directories before the fault
and checks byte counts/hashes afterward. A saved ID cannot recover a deleted
provider database or a missing program.

## Verification boundaries

The isolated fixtures use installed real CLIs and private HOME/provider state.
Only model responses are deterministic loopback test servers; there are no
upstream model calls or reads of personal sessions/configuration.

- Codex 0.169.0: standalone SIGKILL/resume verifies exact structured ID and prior
  user/assistant context. The full Host/holder test kills the real Codex TUI,
  observes automatic recovery, verifies a new PID/UUID with the same provider
  ID and actual saved history visible again, and checks that the model request
  count does not increase during recovery (no prompt replay).
- Claude Code 2.1.325: seeded conversation, pending-turn SIGKILL and explicit
  full-ID resume retain prior user/assistant context through the real CLI.
- Copilot 1.0.112-0: the same sequence preserves committed history. The unfinished
  interrupted user turn is rolled back by Copilot and is not replayed. A missing
  UUID fails without creating a conversation or issuing a model request.
- OpenCode and additional tools can register a versioned `ResumeAdapter`. No
  unverified full-TUI command is guessed from another mode's similarly named
  flag. Command planning does not prove that a provider's state files exist.

All three standalone resume fixtures also reject a missing canonical UUID
without issuing a model request or creating a replacement conversation.

All commands run through Bazel. The provider directory lists its standalone
manual CLI targets and required executable environment variables. Host targets
include `host_test`, `agent_resume_codex_test`, `agent_resume_ssh_test`,
`hard_kill_test`, `hard_kill_ssh_test` and `agent_resume_vm_test`. See [the failure matrix](FAILURE_MATRIX.md)
for exact default-persistence cases and [the terminal acceptance matrix](../lab/ACCEPTANCE.md)
for native/browser vertical resizing and wider stress boundaries.
