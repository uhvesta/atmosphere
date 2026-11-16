# Live terminal mirror

Build and publish a milestone through Bazel:

```sh
export PATH=$PWD/.tools/bin:$PATH
bazel run -c opt //apps/humata2/terminal/demo:live
```

The launcher prints the authenticated localhost URL, starts a persistent native host if needed, and keeps its private state in `~/.humata-terminal-demo`. Keep the URL private: its fragment grants control of your terminals. No token is baked into browser assets. The private `viewer.url` file holds the same URL for the Bazel browser acceptance runner (`TERMINAL_VIEWER_URL_FILE`).

Run this same target after each verified milestone. It publishes versioned browser assets and atomically updates the entry point while retaining old assets for already-open pages. Refresh the page to use the new build; the host and live terminals keep running. Rebuilding the server binary does not replace an already-running host. Server upgrades need an explicit graceful restart with the same state folder; they must be tested separately from browser refresh.

Use `--state DIR --port PORT` for a separate demo. A trusted `--profiles FILE --start local --start ssh` configuration can start local and SSH-backed terminals in **one local server**. The browser and native GPUI viewer connect to that server and render those same terminal sessions. SSH credentials and host processes remain on the server side. See [host configuration](../host/README.md).

The live target builds the real shared GPUI Wasm component and native host. A built bundle alone is not browser acceptance: run the Bazel browser interaction test, inspect its screenshot, and verify both local and SSH terminal paths before reporting that milestone complete. The URL is local to the machine running the host; access from another machine uses an authenticated forward to that host.

Open the matching desktop viewer against the same host (run the `_fast` target to use the prototype's engine):

```sh
bazel run //apps/humata2/terminal/client:native_fast -- \
  --url ws://127.0.0.1:8741/ws \
  --token-file "$HOME/.humata-terminal-demo/browser.token"
```

Use the token path belonging to your chosen `--state` directory. Additional viewers mirror the same sessions and start read-only; **Take control** explicitly transfers the terminal's input and resize lease.

For subsequent milestones, stage and test a candidate before promoting the stable URL:

```sh
bazel test //apps/humata2/terminal/demo:milestone_test \
  --test_env=TERMINAL_DEMO_STATE="$HOME/.humata-terminal-demo" \
  --test_env=TERMINAL_BROWSER_INPUT=1 \
  --test_env=TERMINAL_BROWSER_MIRROR=1 \
  --test_output=errors --nocache_test_results
```

Use a dedicated test demo containing shells: the input checks execute fixture commands in its visible terminals. The runner requires an already-running host, stages versioned assets at a private candidate URL, runs headless Chrome input/reload/mirror checks, and updates the stable entry point only after success. On failure, the previous entry point remains available. `TERMINAL_DEMO_PORT` selects a nondefault port; `TERMINAL_LAB_NODE` selects the installed Node22+ executable. Optional `TERMINAL_EXPECT_SESSION_OS` maps stable terminal IDs to exact `uname -s` results. Screenshots and diagnostics land in Bazel's undeclared test outputs.

Chrome runs with a disposable profile and, on macOS, the testing flags `--use-mock-keychain --password-store=basic` so testing does not request or modify the user's keychain. Cancelling a test stops only that owned Chrome process.

When the shared compiler server is busy, `publish_built` can stage or promote **already Bazel-built** `--host` and `--bundle` paths through an isolated Bazel output base; it always requires a healthy existing host. Pair this with `web:browser_runtime_test`, which tests the deployed candidate without rebuilding it. Set an isolated `--symlink_prefix` so runtime checks do not replace the workspace's normal `bazel-bin` links. The regular `milestone_test` remains the default build/test/promote workflow.

The live Debian demo uses that VM user's actual login shell (`/bin/bash`, verified from its account), with its normal remote startup files. The matching Oh My Zsh/`af-magic` screenshots were deliberate acceptance fixtures selected through an isolated remote `ZDOTDIR`; the component does not copy the Mac user's shell configuration. Personal dotfiles, command history and private keys are not copied to the VM.

For explicit non-UI text delivery to a dedicated demo terminal:

```sh
bazel run //apps/humata2/terminal/demo:send_text -- \
  --state "$HOME/.humata-terminal-demo" --uuid TERMINAL_UUID --file /private/tmp/terminal-input.txt
```

Input files contain the exact bytes to send, including any desired Enter (`\r`). An acknowledgement means the bytes reached the PTY, not that a command or agent completed. The tool never automatically resends uncertain input. `--take-control` explicitly transfers an existing viewer lease; `--expect REGEX` optionally waits for derived output. Avoid matching the input's own echo when checking execution.

Use the durable UUID from session discovery when addressing a terminal from a
tool. `--id NUMBER` remains available for a host-scoped numeric route; supply
exactly one addressing option. The command speaks protocol3. Native embeddings
can instead use the typed [TerminalHandle API](../README.md#identity-names-and-programmatic-input),
including names, attributes and separate queued/written receipts.

## Isolated browser stress fixture

`fixture_runtime_test` starts its own host on an available loopback port with
three disposable local shells. It never sends acceptance commands into the live
demo. First build the host, holder and browser bundle through Bazel (`-c opt` for the
browser bundle), then pass
their absolute artifact paths through `TERMINAL_TEST_HOST`,
`TERMINAL_TEST_HOLDER` and `TERMINAL_TEST_BUNDLE`:

```sh
bazel test //apps/humata2/terminal/demo:fixture_runtime_test \
  --test_env=TERMINAL_TEST_HOST=/absolute/bazel-bin/path/to/terminal_host \
  --test_env=TERMINAL_TEST_HOLDER=/absolute/bazel-bin/path/to/humata2_holder \
  --test_env=TERMINAL_TEST_BUNDLE=/absolute/bazel-bin/path/to/bundle \
  --test_env=TERMINAL_LAB_NODE=/absolute/path/to/node \
  --test_env=TERMINAL_BROWSER_TAB=1 \
  --test_env=TERMINAL_BROWSER_MEMORY=1 \
  --test_output=errors --nocache_test_results
```

The optional memory acceptance generates approximately10k retained rows, changes
tabs through real browser input, waits for the idle policy, checks replica release
and restores a fresh view before typing again. Shells are explicitly stopped
through the protocol before the fixture host exits. Cleanup failures preserve the
private fixture state and report its path for recovery. Set
`TERMINAL_TEST_SSH_CONFIG` to the trusted `h2-terminal-c1` SSH config to replace
the second shell with a real Debian Bash session. Its unique remote scratch
directory is removed only after the fixture's shells are confirmed stopped.
The VM and the live demo's remote holders stay running.

`TERMINAL_TEST_CLI=1` also verifies actual UUID-addressed delivery through the
CLI before browser interaction. `TERMINAL_TEST_LINUX_PROOF=1` runs the prepared
isolated Linux browser after the Mac phase succeeds, using WebGL/SwiftShader.
`TERMINAL_BROWSER_RELOAD=0` skips the reload step for diagnosis; its output
explicitly records that reload was not checked. Default acceptance includes it.
Browser WebSocket diagnostics retain frame types, routes and lengths, without
authentication tokens or input payloads.
