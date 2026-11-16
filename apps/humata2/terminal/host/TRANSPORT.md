# Persistent SSH transport and the Mosh decision

The host keeps SSH as its transport and automatically reattaches to the existing remote holder after a connection loss. It does not ship or automatically install Mosh. A remote holder owns the PTY/process; the native host owns the authoritative Ghostty VT; native and browser GPUI clients render that state. Reconnection never launches a replacement shell or resends uncertain input.

## Why stock Mosh is not this transport

Mosh authenticates using SSH, then uses UDP to synchronize terminal screen state and provide predictive local echo. Its state synchronization may skip intermediate display updates. It does not provide SSH port forwarding, and its FAQ describes visible-screen synchronization rather than complete scrollback. These are documented design choices, not a defect in Mosh. See the [official technical overview](https://mosh.org/#techinfo), [official FAQ](https://mosh.org/#faq), [project README](https://github.com/mobile-shell/mosh/blob/master/README.md), and [research paper](https://mosh.org/mosh-paper-draft.pdf).

Our protocol carries ordered binary holder output/control frames and opaque full Ghostty checkpoints, including history, parser continuation, modes and hyperlinks. This is an inference from those documented Mosh semantics and our protocol: putting those bytes through a stock Mosh terminal would introduce another terminal emulator and lose the required byte-stream contract. Browser HTTP/WebSocket access would still need its separate authenticated forwarding path. A custom SSP object/binding is a different implementation project, not an interchangeable `ssh` executable.

A user may configure an ordinary terminal program to launch an independently installed Mosh client. That is a separate program/session, not the host's managed SSH endpoint or a verified replacement for our scrollback/checkpoint path. No Mosh package is installed merely to expose an unused option.

## What reattachment guarantees

On SSH failure, the host disables live input while preserving its current display and durable terminal UUID. It aborts and awaits old transport tasks, then processes an ordered retirement barrier after their already-queued acknowledgements. Unacknowledged native tickets and viewer inputs become uncertain; they are never declared written or automatically resent. Viewer sequence high-water marks survive reconnect.

The host retries attachment to the same holder endpoint and namespace, starting from its last parsed output offset. Attempts continue while the host is alive, with backoff growing from one second to a maximum of thirty seconds. There is one attempt per terminal and at most four concurrent attempts globally; four is not an attempt-count limit. Local terminals and metadata remain usable while SSH attempts run because an opaque prepared attachment capability does not borrow the global SessionStore.

A successful handshake keeps the same VT and metadata rather than creating a new process. Old writer acknowledgement maps are discarded before new transport tasks start. Retained bytes first hydrate the authoritative VT; viewers receive a fresh atomic checkpoint before their input becomes live again. This preserves parser state and avoids replaying historical clipboard/bell effects into clients. New terminal queries emitted while disconnected can be answered after their retained output reaches the host.

Explicit stop, known process exit, irrecoverable output gaps, ambiguous terminal-reply delivery and unconfirmed resize state stop automatic reattachment. Those cases preserve the last valid display and require explicit recovery or a new session. Shutdown cancels and awaits connection jobs and drops queued attachment results before releasing ownership. Attach never calls process creation.

## Boundaries

This provides process continuity and safe reattachment, not Mosh's UDP roaming, predictive local echo or frame pacing. A fresh SSH connection still needs reachable networking, unchanged host-key trust and usable authentication. Re-authentication can fail while an SSH agent is locked. We do not change keys, SSH profiles, firewall rules or personal shell files.

The remote holder retains a bounded in-memory output tail, currently 16 MiB. If more output is lost than it can retain, the host refuses to invent missing terminal state. A remote program may wait for terminal-query replies while the host is disconnected. For a canonical VT that must keep answering queries during a laptop outage, run the native host on the process machine and reconnect the viewer through its authenticated forward instead. A process killed on the remote machine, a remote reboot, or a stopped holder cannot recover its process memory from a screen checkpoint.

The test below replaces an owned TCP forwarding path; it does not change the machine's Wi-Fi, NAT, VM interfaces or unrelated SSH sessions. It therefore verifies outage/reattachment and route replacement, not a physical network handoff or speculative-echo performance.

## Verification

```sh
bazel test //apps/humata2/terminal/host:ssh_reconnect_test \
  --test_env=TERMINAL_LAB_SSH_CONFIG="$HOME/.lima/h2-terminal-c1/ssh.config" \
  --test_output=errors
```

The dedicated-VM fixture uses a private SSH configuration that includes the trusted existing configuration and routes only its own connection through a local TCP proxy. The original host-key identity is retained. It cuts that path, then restores access through a different proxy without restarting the VM or remote process. It waits for at least five actual rejected SSH connections before restoring the route, using only a private test backoff to keep the test bounded in time. The fixture checks the same UUID, PID, cwd and environment; offline output and an unanswered DSR query; metadata responsiveness while retrying; lost native and viewer acknowledgements after inputs executed once; no automatic resend; duplicate viewer sequence rejection; and new input after reconnection. Fixture shells are stopped explicitly before cleanup. The manual fixture additionally requires the system netcat for its private ProxyCommand.

Verified gate76867 passed 60 Host cases (three manual SSH cases ignored in that target), the public SDK consumer, existing SSH restart and sibling tests, and the prolonged-outage SSH test. The latter completed in 34.72 seconds and observed at least five failed connections before recovery. The host binary was built in the same invocation. Gate95291 previously passed the same behavior with production backoff; the extended test changes no production retry policy.
