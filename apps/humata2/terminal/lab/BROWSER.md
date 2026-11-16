# Browser feasibility for the generic terminal

Implementation checkpoint, 2026-11-15. The browser is a mirror of the desktop workspace: **one local native server owns local PTYs and connects to remote holders over SSH; desktop and browser render the same server sessions**. The browser has no direct PTY, SSH or host-filesystem access. Multiple viewers need an explicit input/resize controller so different viewport sizes do not race.

The protocol3 native host and same-server local/SSH restart, sibling and automatic reconnect tests pass. The actual shared GPUI Wasm component paints and accepts physical browser input on macOS and Debian Linux; no simulated terminal renderer is involved. The full optimized fixture passes: Mac reload,12k output, idle replica parking/wake, mirrors/control transfer and panning; Linux local/SSH Tab, RGB, OSC8, find/focus, physical Unicode/multiline clipboard and reload. The VM remains721MiB/no swap. Debug builds OOM-killed Chromium during reload; the same source with `-c opt` produces24.3MB Wasm and passes the unchanged fixture. This is bounded acceptance, not universal platform or memory certification. See [the acceptance matrix](ACCEPTANCE.md). The live host has been upgraded to protocol3 while preserving both original shells; matching optimized assets passed live-page paint/discovery and are promoted.

The reusable single-terminal API is `terminal_replica::Replica` (`client:terminal_replica{,_fast}`), independent of the demo's `RemoteWorkspace`, fonts, socket and tabs. Supplied sinks deliver ordered wire updates; application policy controls identity binding, visibility, checkpoints, hooks and optional memory saver. Both demo frontends use those same entities. Native/Wasm binary checkpoint tests cover ABI differences, unfinished sequences and repeated valid/malformed restore/free cycles; those tests alone do not certify browser interaction.
## Inactive-view memory saver

The frontend can release an inactive tab's VT replica and rendered-row caches while the persistent host keeps the PTY, authoritative parser and bounded history alive. The parser must keep running: a background TUI may issue terminal queries and wait for replies. Refocusing obtains a fresh authoritative snapshot rather than replaying cached frontend input. Existing compressed checkpoint policy provides disk recovery separately; parking a view does not introduce another raw transcript.

The implementation parks unselected tabs after 30seconds, with an off switch. Selection, find, composition, pending paste/input, scroll position and local panning pin a view until that interaction is finished. An ordered detach acknowledgement separates old in-flight snapshots from a fresh subscription. Focused component/policy/barrier and Host zero-viewer query tests pass; actual Mac acceptance exercises12k output, release and fresh wake. Wasm allocator pages may remain at their previous high-water mark after free; released replicas make that memory reusable but do not guarantee lower browser process RSS.

## Reuse the existing GPUI view

The prototype already has a `wasm-bindgen` entry point, `WebPlatform`, bundled browser fonts and `run_embedded` in `ui-prototype/src/lib.rs`. Its `src/terminal.rs` mounts `TerminalView<DemoGrid>`; those process outputs are simulated. `terminal/view` already separates `Grid` and `TerminalHost` from files, sockets and process management.

The exact pinned **libghostty-vt 1.0.1** has an official Wasm build and allocation/ABI helpers: [installation](https://lib.ghostty.org/vt/installing.html), [Wasm API](https://lib.ghostty.org/vt/group__wasm.html), [library boundary](https://lib.ghostty.org/vt/index.html). This is terminal state, parsing and input encoding, not Ghostty's native Metal renderer.

Implemented integration boundaries:

- The pinned Wasm VT archive and32-bit ABI helpers are built through Bazel; native/Wasm checkpoint conformance is a separate gate.
- `terminal/session` retains Unix files/flock, processes, holder sockets and OpenSSH on the server. Browser code cannot access them directly.
- `terminal_replica::Replica` and the lower-level pane/view accept caller transport and hooks; the native and browser apps supply adapters.
- The Web bundle, bindgen tool, server, interaction fixture and Linux runner all have Bazel targets. Do not invoke the old prototype's Cargo script.
- Runtime platform shortcuts preserve focused terminal Tab/Shift-Tab and browser clipboard behavior. A bounded FIFO orders async clipboard reads before later keystrokes; canceled/refused pastes cannot release a stray Enter.
- Browser output buffering uses a byte budget and per-frame drain bound. Diagnostics retain types/routes/lengths, not auth tokens or input payloads.

## Session transport and recovery

A browser frontend needs a host/server gateway for real PTYs and SSH. The gateway owns attachments and bridges input, output offsets, resize, exit and flow-control acknowledgements over an authenticated WebSocket. Browser tab loss should detach its frontend, not stop the holder. Reconnect catches up from known offsets; renderer state restoration must match the chosen browser/server VT ownership model.

The production `api/proto/humata/v1/terminal.proto` already defines `TerminalService.Attach` messages for Open/Input/Resize/OutputAck, output offsets/windows, read-only viewers and state notices. Prefer adapting that protocol over inventing a second long-lived terminal API. Stock gRPC-Web is not a bidirectional-streaming drop-in for Attach; a WebSocket-to-native-tonic adapter is the practical bridge. [Official gRPC-Web streaming support](https://github.com/grpc/grpc-web#streaming-support).

The current daemon's retained-byte “snapshots” are not complete VT snapshots. Neither a raw byte tail nor a browser framebuffer guarantees reconstruction of every arbitrary TUI. Preserve ordered output/resize history or true terminal checkpoints, and test recovery explicitly. Browser storage cannot preserve a running shell after its host dies.

Use existing authenticated host identity and attach grants; validate WebSocket origins and bound output/input queues. A local browser app still needs a local native gateway; a hosted page needs a reachable authenticated gateway on the machine that owns the session. Do not embed long-lived host credentials in page bundles or expose unauthenticated terminal ports.

## Alternative frontends and engines

| Path | Tradeoff |
|---|---|
| GPUI browser view + Ghostty Wasm | Closest to one reusable GPUI component and the same engine behavior. Needs Bazel/browser integration and real browser acceptance/performance work. |
| GPUI browser view + server-side Ghostty Grid | Smallest initial engine-port proof; needs a well-defined grid transport, server-side selection/scroll policy and network-latency testing. |
| xterm.js + existing native session backend | Mature browser-specific frontend, but a second renderer/input implementation to maintain beside GPUI. Useful as a comparison or fallback. |
| Alacritty core in Wasm | Released 0.26.0 still includes native TTY/event-loop dependencies; current upstream adds Wasm cfg exclusions. It does not supply a browser renderer or PTY, so switching engines does not remove the main browser work. |

Primary references: [xterm.js capabilities/addons](https://github.com/xtermjs/xterm.js), [link handling](https://xtermjs.org/docs/guides/link-handling/), [flow control](https://xtermjs.org/docs/guides/flowcontrol/), [serialization addon](https://github.com/xtermjs/xterm.js/tree/master/addons/addon-serialize), [WebGL lifecycle](https://github.com/xtermjs/xterm.js/tree/master/addons/addon-webgl), [released Alacritty source](https://docs.rs/alacritty_terminal/0.26.0/src/alacritty_terminal/lib.rs.html), [Alacritty upstream](https://github.com/alacritty/alacritty/blob/master/alacritty_terminal/src/lib.rs).

Browser clipboard, IME, focus and keyboard behavior need their own tests under browser activation/permission rules. Test WebGL/WebGPU context loss and restoration, resize, 10k+ scrollback, backpressure, background tabs, tab reload, disconnected/reconnected sockets and multiple viewers. Native Cmd+C/V results do not certify browser clipboard behavior. [Clipboard API](https://developer.mozilla.org/en-US/docs/Web/API/Clipboard_API).

## Pinned browser compiler opt-in

The GPUI Fast browser dispatcher currently resolves `wasm_thread 0.3.3`, which uses the nightly-only [`stdarch_wasm_atomic_wait` intrinsic](https://doc.rust-lang.org/stable/core/arch/wasm/fn.memory_atomic_notify.html). The experimental Bazel Web index scopes `RUSTC_BOOTSTRAP=wasm_thread` to that exact crate/version and restricts enabled unstable features with `-Zallow-features=stdarch_wasm_atomic_wait`. Native dependencies and other crates receive no bootstrap opt-in. This is not a generally stable compiler API; reverify it when changing the pinned Rust or dispatcher version. A pinned nightly Web toolchain or upstream removal of this dependency is the longer-term alternative. [Rust's documented crate-scoped bootstrap behavior](https://doc.rust-lang.org/unstable-book/compiler-environment-variables/RUSTC_BOOTSTRAP.html).

The current browser uses the dispatcher's single-thread mode; it does not enable shared-memory worker pools or require cross-origin isolation headers for this proof.
