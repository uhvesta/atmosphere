# humata2_clients

The dependency clients Humata 2's features use, one trait per outside
service:

| Trait | What for |
|---|---|
| `Daemon` | Everything the app asks the daemons, and the event stream they push |
| `Terminals` | Typing into a session's terminal tabs |
| `Clock` (tca) | Latency, streaming pace and save intervals |
| `Clipboard` | Copy actions reducers take |
| `FileSystem` | Files the app writes itself (bug reports, prompts) |
| `Keychain` | Host variables' values and tokens |
| `Storage` (tca) | The app's own state between launches (PER-25) |

`Clients` bundles one of each; features receive it and never name an
implementation.

## Fakes only, for now

Every implementation here is a fake with deterministic dummy data
(`fixture::standard`, and `fixture::large` for thousands of sessions):
hosts online and offline, channels with several repositories, sessions in every
state with forks, Inbox requests of every kind, diffs with comments, boards
with dependency edges, files, transfers, trash, pull requests, specs, a
journal and prepared views. `FakeDaemon` simulates latency and a streaming
agent on whatever `Clock` it is given (a `TestClock` in tests), fails calls
on request (`fail_next`), and models reconnecting a host: each of its
sessions comes back reattached, resumed, or recoverable with what is needed.
Its event queues are bounded; a subscriber that falls behind gets `Resync`
and reloads instead of an unbounded backlog (TR-8).

## Real data later

The real clients will implement these same traits over `api/proto` (gRPC
over the SSH or stdio link) and the macOS keychain, clipboard and file
system. The desktop app builds `Clients` in one place; swapping the fakes for
live implementations there changes nothing in `features/` or `ui/`, which
only see the traits and the plain values in `model` and `Workspace`. Those
values stand in for `humata2_model` and the generated messages until they
exist (D55).
