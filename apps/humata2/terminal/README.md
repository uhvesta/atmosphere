# Reusable GPUI terminal

This experiment provides a standalone GPUI terminal component and optional
runtime adapters. Native and browser demos use the same component. Main-app
adoption and a separately released public package remain integration work.
See [acceptance](lab/ACCEPTANCE.md) for verified behavior and
[handoff](lab/HANDOFF.md) for the current implementation state.

## Component contract

- Render the actual terminal pane without requiring the demo's header, tabs,
  split groups or notifications. The consuming application owns its layout.
- Accept an input/output transport and compatible supplied checkpoint. Preserve
  ordered output/resize and disable uncertain input until resynchronized. Never
  resend input merely because a view reconnects.
- Keep keyboard input scoped to the focused terminal. Tab and Shift-Tab belong
  to that terminal while it is focused; ordinary form navigation still works
  outside it. Mouse selection, application mouse reports, scroll and links
  respect the terminal's visible bounds.
- Make memory saver opt-in. An inactive, unpinned replica may release its VT and
  rendering caches. Its host keeps parsing and answering queries while the
  process runs. Refocus requests a fresh snapshot through an ordered detach/open
  boundary. Existing disk recovery is a separate configured host policy.
- Expose typed events for the host application to consume. Notifications, bells,
  links, agent lifecycle and shortcut requests do not require OS notifications
  or application-specific chrome.

The low-level pane is `terminal/pane::TerminalPane` with its `Pty` trait and
events. The `terminal_replica` crate adds single-session replica lifecycle over
the transport-neutral `terminal/wire`. Build
`//apps/humata2/terminal/client:terminal_replica` or its `_fast` variant; the
demo also reexports the same API as `terminal_client::replica`. It takes supplied
`ReplicaConfig` and `ReplicaSink`; it installs no socket, fonts, global theme or
workspace chrome. The standalone `client:component_test` and
`client:component_fast_test` exercise that standalone library directly without
`RemoteWorkspace`. The existing native/browser demo uses these same entities.

The embedding app authenticates its transport, supplies matching session
metadata through `bind_connection`, and routes ordered updates through `apply`
with the connection epoch. It explicitly controls visibility and optional idle
memory policy. Raw `ReplicaCheckpoint` export/restore leaves storage and
compression to the caller. A restored display remains read-only until fresh
authenticated host state and a controller lease arrive. Pending input is never
stored in a checkpoint or replayed on reconnect. Subscribe to `ReplicaEvent`
for state, focus, pane/provider hooks, shortcut requests and input receipts.

## Identity, names and programmatic input

The native host exposes UUID-addressed handles. Persist `terminal.uuid()`;
numeric `route()` values are scoped to their owning host. `host.terminals()`
discovers records, and `host.terminal(uuid)` obtains a handle. UUID values, names and
bounded string attributes survive metadata reloads. Existing records receive
stable UUID values during migration.

```rust,ignore
let terminal = host.terminal(saved_uuid).await?;
terminal.set_name("Investigate PAY-9".into()).await?;
let queued = terminal
    .inject_text("git status".into(), InjectionOptions::raw())
    .await?;
let ticket = queued.ticket().clone();
queued.written().await?;
```

This queues text without appending Enter. `InjectionOptions::paste()` respects
the terminal's paste mode; `submit: true` explicitly appends submission input.
Supplied newlines still have the meaning assigned by the running terminal
application. A queued receipt means accepted by this host; a written receipt
means acknowledged by the PTY holder. Neither means the shell command or agent
task completed. Uncertain delivery is a typed error carrying its ticket and
must not trigger automatic resend. Input is bounded before enqueueing. Another
controller's lease is refused unless the caller explicitly chooses
`ControlPolicy::TakeControl`.

`SessionInfo` exposes the effective `title`, `automatic_title`, optional
`name_override` with `Manual` or `Programmatic` provenance, and `attributes`.
`set_name()` is a programmatic override; `set_name_with_source()` records an
explicit manual override. Explicit renames replace the previous override,
while runtime title updates preserve it. `clear_name()` restores the latest
automatic title. `set_attrs()` replaces the bounded attribute map atomically.
The embedding application owns authorization and any stricter naming policy,
so a tool wrapper can map its own task IDs and permissions onto these handles.
Metadata is not a place to store credentials or raw prompts.

The [public native consumer](demo/native_api_test.rs) exercises discovery,
rename, attributes, input with separate submission, fresh sibling creation and
confirmed process exit using these public APIs. Its Bazel target is
`//apps/humata2/terminal/demo:native_api_test`. Host tests also exercise native
injection into an actual SSH terminal.

## Recovery requirements

A surviving local/SSH holder can reconnect to its original process. Keep its
private workspace metadata, endpoint identity, matching holder protocol and
compatible terminal checkpoints. A saved display can be restored after the
process disappears, but cannot recreate arbitrary process memory. Agent
conversation resume additionally needs an explicit provider session ID, working
directory and any provider-owned state files, with the provider installed on the
same machine. Missing or ambiguous recovery must return a useful error and keep
the last verified state inspectable.

Checkpoint encoding, compressed disk size, retained live history and restore
allocation have separate limits. History persistence is configurable, including
off. Ordinary input, environment, launch arguments and notification bodies are
not additional persisted transcripts. See [host configuration](host/README.md).

## New terminal from an existing context

The requested logic-only sibling operation starts a fresh process and allocates
a new terminal ID. It takes the source's local/SSH endpoint and latest known
working directory, then uses a configured fresh shell or agent launch profile.
It does not copy pending input, history, provider conversation IDs or process
memory. Unavailable directories or missing launch profiles produce errors; they
do not silently switch machine or working directory. Use
`TerminalHandle::spawn_from(SpawnFromOptions)` for this logic without a Cmd+T
user interface. `RequireObserved` is the default cwd policy; callers may
explicitly choose `AllowLastKnown` or supply a directory with `Explicit`.

Current-directory reporting is explicit: OSC7 or caller-supplied metadata can
update it. A shell that reports no directory changes supplies only a launch/last
known path; that must not be presented as a verified live cwd. Integration must
not discover it by typing `pwd` into an arbitrary running TUI.

## Optional shortcuts and provider hooks

The consuming app may install actions for Cmd+T (new terminal), Cmd+K (command
palette), Ctrl+Tab (next) and Ctrl+Shift+Tab (previous). The component emits action
requests; the app owns navigation and palette presentation. Register global
actions at the application root when they should also work outside terminal
focus. Installing those actions is explicit so other TUIs can retain their key
bindings when the application does not claim them.

Native application bindings and browser accelerators differ. Browser-reserved
Cmd+T or Ctrl+Tab may never reach the page; the web embedder can offer alternative
bindings for the same logical actions. The component does not force fullscreen
or keyboard lock.

Provider adapters consume supplied structured hook payloads, not guessed IDs
from painted terminal text or scanned conversations. The normalization
boundary supports versioned Claude Code, Codex, Copilot and OpenCode schemas and
an adapter extension point. A trusted launcher binds events to a terminal;
ordinary viewers cannot impersonate a provider. Explicit session metadata may
update recovery hints, while transient notification events are exposed to the
embedding app. See [provider adapters](provider/README.md) and
[host configuration](host/README.md) for the implemented native hooks.

The desktop app has **Settings → Notifications → Permission request alerts**.
Its saved `permission_request_alerts` preference defaults off. When enabled,
each new pending permission request produces one extra in-app alert; duplicate
events and already answered requests do not repeat it. This controls alerts only.
**Settings → Agents → Ask for permission in the Inbox** separately chooses
between Humata's Inbox and the agent's own terminal prompt. Notification delivery
remains application policy, separate from the reusable component's provider hooks.

The real local OpenSSH fixture is documented in
[SSH demo setup](platform_test/SSH_DEMO.md) and automated by the repository-root
`terminal-ssh-demo.sh`. It uses a dedicated no-mount Debian Lima VM.
