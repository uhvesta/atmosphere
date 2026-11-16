# Durable terminal metadata

Every saved terminal has a canonical durable UUID alongside its internal
numeric route. UUID values are generated from fallible native OS randomness before
the descriptor is saved. A failed launch keeps the reserved UUID; an explicit
new instance receives a new UUID. Moving, attaching or restarting the host
does not change identity. `session_by_uuid` resolves wrapper-facing identity
without launching a replacement process.

Naming has two layers:

| Update | Result |
| --- | --- |
| `set_automatic_title` | Changes the latest runtime title; an explicit override remains visible |
| `set_name_override(..., Manual)` | Sets an explicit user name |
| `set_name_override(..., Programmatic)` | Sets an explicit caller/agent name |
| `clear_name_override` | Displays the latest automatic title again |

Explicit override updates use last-writer-wins semantics. App authorization
and any caller-specific manual-name protection policy belong to the wrapper.
Names contain 1–512 UTF-8 bytes without control characters. `display_title`
returns the effective name, while the original launcher label remains in
`SessionSpec.title`.

`set_attributes` atomically replaces a caller-supplied string map: at most
32 entries, 64 bytes per key, 4096 per value and 32 KiB total key/value bytes.
Empty values are supported; keys are nonempty and control characters are
refused. These are persisted non-secret wrapper attributes. Callers choose
them explicitly; this component never infers attributes from argv,
environment, terminal input or provider transcripts.

Workspace schema 3 stores UUID values, automatic names, explicit overrides and
attributes. Schema 1/2 migration preserves numeric routes, holder locations,
layout and namespace, assigns missing UUID values and saves once atomically before
returning the store. Legacy automatic labels are bounded and stripped of
control characters; the launcher specification is preserved. Schema 3
requires unique canonical UUID values and rejects malformed identities instead of
regenerating them. Older writers reject schema 3 rather than discarding its
identity fields.

Metadata updates validate bounds before saving, and a failed atomic write
restores the previous in-memory state as well as preserving the prior file.
The existing 16 MiB workspace and 256-terminal limits still apply.

```sh
bazel test //apps/humata2/terminal/session:metadata_test \
  //apps/humata2/terminal/session:session_test \
  //apps/humata2/terminal/session:session_unit_test --test_output=all
```


For a host that owns its VT state, `prepare_live_attachment(id)` creates an
opaque, cloneable reconnect capability for a memory-only store. Its owned
`attach_from(offset)` future can run independently of the workspace, so an
SSH reconnect does not borrow other terminals' metadata or delay local
session operations. All clones retain the exclusive attachment lock. The
capability stores no launch arguments and never starts a replacement process
or replays terminal input. Journaled views continue using `attach` instead.
