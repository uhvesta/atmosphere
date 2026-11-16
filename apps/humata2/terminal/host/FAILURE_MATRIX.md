# Hard-kill persistence matrix

The process-kill tests launch a real `terminal_host` subprocess and communicate through its
authenticated WebSocket gateway. They use private state directories and uniquely
identified shell/holder processes. The SSH cases use OpenSSH in the dedicated,
no-mount `h2-terminal-c1` Lima VM; they never restart the VM or kill live demo
processes.

| Fault | Verified outcome |
| --- | --- |
| Local host receives SIGKILL, local holder survives | Restarting the host preserves terminal UUID, shell PID, working directory and shell environment. Output produced while the host was absent appears after hydration. Input is not replayed. |
| Local host receives SIGKILL, SSH holder survives | The same remote holder/shell is reattached, with the same UUID/PID and offline output. The startup profile does not launch another shell. |
| Owned SSH attachment process receives SIGKILL | The host becomes read-only while disconnected, then reattaches the same remote process. A new input is written once after recovery. |
| Local or SSH holder receives SIGKILL | A previously durable checkpoint remains visible and read-only, with `Unavailable` state. Reopening the host does not silently create a new process. |
| Local or SSH child shell receives SIGKILL | The surviving holder reports a positive `Exited` event. Reopening the host retains that terminal UUID and exit state without relaunching the shell. |

The holder-death case deliberately reports **Unavailable**, rather than claiming
an exit code it did not observe. A failed attachment can also mean an offline
machine, refused SSH authentication or an unavailable socket. It is not sufficient
evidence to launch a replacement agent. Provider conversation recovery is a separate
trusted, explicitly enabled operation using a saved provider session ID; these
tests do not enable it. Saved recovery metadata alone does not authorize a restart.

The fixtures first wait until the checkpoint's recorded output offset covers the
observed marker. Consequently the holder-death test proves disk-backed recovery,
not just a display remaining in an existing viewer's memory. Host-death tests also
check a per-process counter and one launch marker, detecting duplicate startup or
automatic input resend.

Run the three local cases:

```sh
bazel test //apps/humata2/terminal/host:hard_kill_test --test_output=all
```

Prepare/check the dedicated fixture with the repository helper, following
[SSH demo setup](../platform_test/SSH_DEMO.md), then run the four SSH cases serially:

```sh
./terminal-ssh-demo.sh status
bazel test //apps/humata2/terminal/host:hard_kill_ssh_test \
  --test_env=TERMINAL_LAB_SSH_CONFIG="$HOME/.lima/h2-terminal-c1/ssh.config" \
  --test_output=all
```

`TERMINAL_LAB_SSH_HOST` and `TERMINAL_LAB_SSH_HOLDER` may select the trusted fixture
alias and matching installed holder. They default to `lima-h2-terminal-c1` and
`/tmp/h2-terminal-holder`. Coordinate VM use with browser/resize acceptance tests.

Verification on 2026-11-16: all three local cases passed (3.1 seconds), followed by
all four SSH cases (12.24 seconds of Rust test execution). These are actual process
SIGKILL tests, separate from the existing TCP-cut/reroute outage fixture.

## Whole-guest power loss

`agent_resume_vm_test` runs the native Host against a **fresh disposable** Lima VZ
guest: one CPU, 768 MiB memory, a 4 GiB disk, no host mounts, no agent forwarding,
and fresh fixture SSH keys. Personal public keys are not loaded. Its private
`LIMA_HOME` and fixed instance name are checked before every stop/delete action;
it cannot operate on the live `h2-terminal-c1` fixture.

This is a forced guest power-off, not a process-kill substitute. Lima's pinned
[force-stop implementation](https://github.com/lima-vm/lima/blob/v2.2.1/pkg/instance/stop.go#L101)
kills the VM driver. The test compares Linux kernel boot IDs before and after
restart, verifies fsynced conversation/history state and installed executable
hashes survived, and keeps the local Host running throughout. With automatic
resume enabled, the old terminal remains `Unavailable` without a replacement
while the guest is unreachable or its old holder is missing. Only after observing
a new boot ID does the native orchestrator attest process loss and request recovery.

The recovered process has a new terminal UUID, the same exact saved conversation
ID, the same remote endpoint and working directory, and the retained history.
The CLI is a **strict generic fixture implementing Codex's explicit resume argument
contract**, not installed remote Codex or a model service. Separate local real-vendor
CLI proofs are recorded in [agent recovery](AGENT_RECOVERY.md). Across reboot,
process identity is `(boot_id, PID)` because numeric PIDs can be reused.

Build the Linux holder first through Bazel, then pass that existing artifact to
the manual test; the test driver never invokes a nested build:

```sh
bazel build --config=linux_arm64 \
  --extra_execution_platforms=@platforms//host,//bazel/platforms:linux_arm64 \
  //apps/humata2/daemon/holder:humata2_holder
terminal_vm_holder=$(bazel cquery --config=linux_arm64 \
  --extra_execution_platforms=@platforms//host,//bazel/platforms:linux_arm64 \
  --output=files //apps/humata2/daemon/holder:humata2_holder)
bazel test //apps/humata2/terminal/host:agent_resume_vm_test \
  --test_env=TERMINAL_VM_LINUX_HOLDER="$PWD/$terminal_vm_holder" \
  --test_env=TERMINAL_VM_IMAGE="$HOME/Library/Caches/lima/download/by-url-sha256/b4c920677ef3b13bc95061fd4e5c70d4bdc8f46220c6021d58aa054a927d5663/data" \
  --test_output=all --nocache_test_results
```

The image is the already cached, SHA-512-pinned Debian 13 aarch64 image from
2026-10-01. A missing cache fails explicitly rather than starting a large image
download. `TERMINAL_VM_IMAGE` can name another local copy of that same pinned
image. Success deletes the owned VM; failure stops it and preserves the printed
private fixture path for inspection.

The first probe exposed a setup durability error: the freshly copied holder and
fixture executable became zero-byte files after forced power-off, while the
explicitly fsynced conversation/history remained intact. Setup now fsyncs both
executables and their parent directories before launch, then compares sizes and
SHA-256 hashes after reboot. Recovery requires the installed program and provider
state to survive; it does not reconstruct lost executable files.

Final uncached Bazel gate `28349` passed on 2026-11-16: 37.47 seconds of Rust test
execution (38.0 seconds in Bazel). Sanitized evidence is emitted as the test's
undeclared output `vm-powerloss-proof.json`:

| Evidence | Before → after |
| --- | --- |
| Kernel boot ID | `ab68120f-d4df-4121-b805-992611d625a7` → `37c2aea0-75e5-4124-a643-b01617c0b26b` |
| Terminal UUID | `5799f6e3-7f8c-49b9-80f9-ed61b46ec962` → `76465e2c-5fa1-4dbb-96eb-4052068259d5` |
| Agent / holder PID | `1315 / 1312` → `913 / 909` |
| Exact conversation ID | `019a83ae-6368-72f5-90bc-5799b82886a1` unchanged |
| History | `durable before power loss` retained, then `resumed after new boot` appended |

The successful disposable VM was deleted. The earlier failed probe was preserved
stopped; no live demo VM or terminal process was stopped or restarted.
