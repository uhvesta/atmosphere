# Browser terminal proof

This is the **same GPUI terminal component**, compiled to WebAssembly.
JavaScript adapts browser WebSocket APIs and starts the generated Wasm
application; it does not implement a terminal renderer, escape parser or command
runner. Programs, SSH and files remain on the authenticated persistent host. The
browser holds only a VT replica and sends explicit user input.

Build release assets entirely through Bazel:

```sh
bazel build -c opt //apps/humata2/terminal/web:bundle
```

Use `-c opt` for browser releases; the default build is useful for development
but has a larger Wasm module and higher startup memory costs.

`bundle` uses GPUI Fast with a browser-only dependency closure that excludes
native test helpers. It produces a directory containing `index.html`, generated
wasm-bindgen JavaScript/snippets and `terminal_web_bg.wasm`. Serve the resulting
directory with the terminal gateway. Open its HTTP URL with `#token=TOKEN`; the
WebSocket defaults to the same origin’s `/ws` endpoint. Optional `?ws=...`
selects an explicitly configured gateway address. Keep tokens out of shared
screenshots/logs and use TLS for nonlocal deployments.

`wasm32-unknown-unknown` is a registered Bazel Rust cross target. The pinned
Ghostty Wasm archive links into the same Rust Wasm module. `tools.bzl` supplies
the official checksum-pinned wasm-bindgen CLI matching crate version 0.2.127.
The final link strips debug information, and wasm-bindgen removes the large
debugging name section before packaging. No Cargo command or `target/` build
tree is used.

## Real browser acceptance

The manual smoke target launches an isolated **headless** Chrome/Chromium
profile, loads the running gateway, checks real GPUI viewport/state and captures
the actual canvas. With input enabled it clicks each visible live pane using the
widget’s measured geometry and types a distinct marker plus `uname`, then
reloads the same sessions. Optional mirror acceptance opens a second browser
page, transfers control through the visible button, and verifies that input
updates both pages. It types through Chrome’s native CDP input events; it does
not inject a fake terminal message. The viewer exposes read-only
`window.terminalInspect()` diagnostics for acceptance.

```sh
bazel test //apps/humata2/terminal/web:browser_smoke_test \
  --nocache_test_results --strategy=TestRunner=local \
  --test_env=TERMINAL_LAB_NODE=/absolute/path/to/node \
  --test_env=TERMINAL_VIEWER_URL='http://127.0.0.1:8741/#token=TOKEN' \
  --test_env=TERMINAL_BROWSER_INPUT=1 \
  --test_env=TERMINAL_BROWSER_MIRROR=1 --test_output=all
```

Use Node 22+ with built-in WebSocket. `TERMINAL_BROWSER` can select another
Chrome/Chromium binary; `TERMINAL_BROWSER_FLAGS` is an optional JSON argument
list for controlled GPU/software-rendering test configurations. The test writes
`browser-terminal.png` and `browser-proof.json` to Bazel’s undeclared test
outputs. It uses a temporary browser profile, never a personal profile.

For cross-OS proof, run the browser on a different OS from the process host,
through a controlled authenticated network endpoint. The browser package has no
native session/holder dependency and cannot open the host’s PTY, SSH connection
or files. Clipboard APIs require a secure browser context; localhost is suitable
for a forwarded test endpoint, while ordinary remote deployments should use
HTTPS/WSS.

Each input run uses a fresh random marker and checks `uname -s` as the exact
output line after that marker. To verify particular local/SSH routes, set
`TERMINAL_EXPECT_SESSION_OS` to a JSON object mapping stable numeric session IDs
(preferred) or current session titles to `Darwin` or `Linux`.
`TERMINAL_VIEWER_URL_FILE` can read the authenticated URL from a private file
instead of placing it in test command arguments. `TERMINAL_BROWSER_MIRROR=1`
additionally verifies read-only startup, explicit control transfer, output in
both views, and a different viewport size.

`browser_runtime_test` runs the identical browser interaction proof against an
already built and deployed external server. It deliberately has no bundle build
dependency; build and deploy `:bundle` first. This lets runtime acceptance use
an isolated Bazel output base while another checkout gate is active, without
compiling a second copy of GPUI or bypassing Bazel.

Browser acceptance targets carry Bazel’s `external` tag: they always execute
against the current server, rather than reuse a result for an earlier runtime.

`?backend=webgl` selects the public GPUI WebGL backend for graphics diagnosis;
`?backend=webgpu` forces WebGPU, and the default is `?backend=auto`. This changes
only the graphics backend, not the terminal component, state or transport.
Acceptance decodes the compositor's PNG and rejects an all-black result even
when model diagnostics contain terminal text. Runtime console events accompany
success and failure artifacts with the exact authentication capability redacted.

`TERMINAL_BROWSER_STYLE=1` additionally prints unique truecolor and OSC 8 labels
through each real shell and modifier-clicks the link cells to check their URLs.
`TERMINAL_BROWSER_PAN=1` requires mirror mode, emits a shared alternate-screen
probe, pans the smaller read-only viewport to its last column and row, checks
that the owner dimensions did not change, and captures before/after images.
The probe restores the normal terminal buffer even when an assertion fails.

Opt in with `?memory=1` or the header toggle. Memory saver parks unselected
idle tabs after 30 seconds,
releasing their frontend VT and row caches while the host process keeps running.
Visible panes remain resident. Waking requests a fresh checkpoint; no browser
filesystem cache is created. Search, selection, pending paste, composition and
scrolled views remain pinned. The header includes an on/off control, and
read-only diagnostics report resident/parked counts rather than claiming an OS
memory reduction. Browser document visibility does not currently trigger parking.

`TERMINAL_BROWSER_MEMORY=1` adds an optional real-browser idle/wake test against
a dedicated host with at least three sessions and a 10k-line history policy. It
prints dense history through a real shell, selects the third tab, waits for the
30-second timeout, checks resident-count reduction and the host detach
barrier, then refocuses and types again after fresh restoration. It writes
`browser-memory-proof.json` and a parked-state screenshot. This flag never adds
the idle delay to ordinary browser acceptance.

`TERMINAL_BROWSER_TAB=1`, with input enabled, defines a unique shell function in
each visible session and types only its command prefix. It sends physical Tab
and Enter through Chrome and requires the function's unique output. This checks
real shell completion in both local and SSH routes when those profiles are
selected. The focused terminal claims bare Tab/Shift+Tab before GPUI's generic
focus-traversal bindings; input fields elsewhere retain normal traversal.

`TERMINAL_BROWSER_CLIPBOARD=1` is optional and requires an isolated Linux VM
with `TERMINAL_BROWSER_ISOLATED_CLIPBOARD=1`. It seeds only that fixture's
clipboard, presses physical Ctrl+Shift+V, executes the pasted command, selects
its output and presses Ctrl+Shift+C. Clipboard reads return equality booleans;
unknown clipboard text is never logged. Selection is cleared before any memory
test. This flag does not access the Mac user's clipboard.

The browser transport starts its WebSocket after GPUI window initialization,
so slow graphics setup does not consume the host's authentication deadline.
Incoming events use a 64 MiB retained-memory budget including string storage
and per-message overhead. A bounded drain preserves FIFO ordering and accounts
for the events it leaves queued. Tiny output packets do not have a separate
256-message disconnect threshold. A true budget overflow fails explicitly;
uncertain input is never replayed.

```sh
bazel test //apps/humata2/terminal/web:incoming_queue_test \
  --test_env=TERMINAL_LAB_NODE=/absolute/path/to/node
```

The browser harness gives initial GPUI readiness and intentional page reloads
a bounded 60-second startup budget for cold software graphics initialization.
Interactive CDP commands keep their 15-second deadline. Startup and reload
artifacts record each phase and elapsed time. Every selected terminal must load
before input acceptance starts; configured OS routes cannot be silently skipped.
Before physical input the harness brings its own page to the
foreground; it does not force DOM focus or bypass terminal focus checks.

Clipboard acceptance intentionally presses Enter immediately after physical
paste. The component buffers following keyboard/text input behind asynchronous
clipboard reads; the test adds no sleep to hide that ordering requirement.
Rejected reads or unaccepted pastes cancel buffered typeahead without sending
Enter alone. This queue is local RAM only and is not exported in checkpoints.

Successful clipboard acceptance writes a screenshot for each checked terminal
before reloading the page, alongside the exact-byte proof. Application logging
uses the browser dependency closure's Info level; warnings and errors remain
visible without flooding cold shader initialization with debug output.

The browser entry exposes idempotent `shutdown()` for embedded hosts. It closes
viewer sockets and clears queued output, invokes GPUI's public `App::shutdown`
(to release windows, renderers, frame callbacks and observers), then drops the
strong `ApplicationHandle`. It never stops server-owned terminal processes.
Page hide invokes this cleanup; a persisted page-show starts a fresh viewer and
restores state from the host. A canceled asynchronous graphics launch cannot
open a late window. `lifecycle_test` checks launch cancellation and idempotency;
the inline transport test checks closed/stale sockets cannot replay output.
