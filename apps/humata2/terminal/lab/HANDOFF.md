# Handoff: reusable terminal, persistent host and browser mirror

## Session metadata

- Project: `/Users/avestabarzegar/oss/atmosphere-khodr`, branch `atmosphere-khodr`.
- Owner: minion C1. Shared checkout contains many unrelated dirty lanes.
- Latest steering: vertical resizing, SSH/Host/remote failure recovery, exact provider-session resume and conversation forks. Neovim-specific work remains explicitly deferred.
- Publishing is authorized. No attribution trailers. All build/test/bench/lint/format commands use Bazel; never Cargo directly.
- The older main-app/frame goal remains paused and unfinished. This terminal experiment does not close it.

## Recovery milestone (2026-11-16)

- `host/AGENT_RECOVERY.md` is the native/CLI opt-in contract. Default Host behavior
  reattaches surviving holders and never starts replacement agents. Enabled
  profiles recover once after positively confirmed nonzero child exit, using a
  full saved provider ID, same endpoint/cwd and fresh terminal UUID. Startup
  reservation and identity persist atomically before spawn. Never replay input.
- Missing SSH/holder is not proof of child death. Only a trusted native
  orchestrator may attest loss with `resume_agent_after_confirmed_loss`.
  Explicit Stop persists recovery suppression; shutdown drains in-flight spawn.
  A launch receipt is not proof of provider acceptance or turn completion.
- `ProviderBinding::activate_fork(parent, structured_child_start, child_paths)`
  persists and pins the selected child ID, revokes the parent binding and rejects
  late background-parent events. The saved workspace ID wins after a settings
  crash window. Fresh same-machine/cwd terminals remain separate `spawn_from`.
- Real CLI resume and fork/kill/resume proofs pass: Codex 0.169.0, Claude Code
  2.1.325, Copilot 1.0.112-0. All use private provider state and deterministic
  loopback model servers, no personal transcripts or upstream calls. Missing
  UUIDs fail without creating conversations/model requests. Committed history
  survives; unfinished turns remain provider-defined. Parent verification
  excludes child-only messages. Background jobs are not resurrected.
- Broad Bazel gate96422: all 52 nonmanual terminal targets pass (325.2s).
  Final performance scheduling gate12135 passes Host and actual Host+Codex;
  SSH rerun89839 passes9.7s after correcting a mistaken fixture environment
  variable. A final reviewed companion-settings failure fix follows durable
  child identity in memory and revokes the parent; final gate9708 passes
  Host72 active/7 ignored and the real Codex Host recovery (25.0/42.5s).
- Browser fixture53988 passes (142s): optimized GPUI WebGL on Mac and Linux,
  fixed-width heights 800→500→1000→800, actual local/SSH rows and last-row
  pixels/cursor, unchanged PID/UUID/environment, reload, Tab, RGB/OSC8, clipboard,
  mirrors/control transfer, 12k output and memory saver. Native ordinary/Fast
  resize tests and dedicated SSH resize targets pass. A pre-existing Fast SSH
  app golden mismatch occurs before the new phase; it was not re-recorded.
- Whole-machine gate28349 passes `host:agent_resume_vm_test` (37.5s Rust,
  38.0s Bazel): fresh isolated Lima/VZ guest forced off, new kernel boot ID,
  synced conversation/history and identical executable sizes/hashes, no auto
  during outage, native confirmed-loss exact-ID resume into a fresh UUID/process
  at the same cwd. Successful VM deleted. The agent is a strict disk-backed
  fixture; real vendor proofs are separate macOS runs. Sanitized proof is in
  `/private/tmp/c1-terminal-vm-powerloss-final-28349/vm-powerloss-proof.json`.
- First power-loss probe left unsynced installed holder/wrapper as zero-byte
  files while synced conversation/history survived. Fixture deployment now
  fsyncs those files and directories. Stopped failure evidence remains private
  at `/tmp/h2-vm-qFQgaY`; do not start or delete unrelated Lima instances.
  Storage destruction, unsynced data and arbitrary process memory are not
  recovered by the component.

## Current state

The standalone `terminal_replica` component now drives the same actual GPUI terminal in native and Wasm clients. It owns no app chrome, fonts or socket. Supplied transport/checkpoints/hooks, explicit identity/epoch binding, opt-in host shortcuts and opt-in inactive memory saver are documented in `terminal/README.md` and `client/README.md`.

The native host owns local PTYs and SSH connections, authoritative Ghostty VT state/replies, controller leases and bounded private disk checkpoints. Viewers mirror those sessions; a browser has no direct PTY/SSH/filesystem access. `TerminalHandle` exposes durable UUID, names/attributes, explicit raw/paste injection with queued/written/uncertain receipts, stop, and a fresh sibling on the same endpoint/cwd. No receipt means agent completion, and uncertain input is never replayed.

The three focused goals have direct evidence:

1. Ordinary SSH copy/paste passes native tests on both GPUI engines and physical isolated Linux browser tests. Actual drag/copy produces plain Unicode with hard newlines; pasting the actual copied value back proves exact remote bracketed UTF-8 bytes. Native tests use isolated headless frontend clipboards; browser tests never touch the personal macOS clipboard. Neovim clipboard is not claimed.
2. SSH automatically reattaches the same remote holder after repeated failed connections. An owned TCP proxy cut/reroute test proves the same UUID/PID/cwd/environment, offline output/query recovery, responsive metadata and lost-ACK uncertainty without input replay. Stock Mosh is not bundled: visible-screen synchronization cannot carry the ordered holder/checkpoint stream. Sources and precise limits are in `host/TRANSPORT.md`.
3. Extra permission-request alerts have a saved local toggle, default off, under Settings → Notifications. They never answer, remove or reroute requests. The other lane's D199 Settings → Agents → Ask for permission in the Inbox independently leaves future asks to the agent's native prompt when off; it never implicitly allows. Already-held Inbox asks remain pending. Preserve that lane's implementation when staging.

## Verification

- Final post-format gate74639: `bazel test //apps/humata2/terminal/... --test_tag_filters=-manual --test_output=errors --jobs=2`, all50 targets pass (175.557s). Includes Component15×2, View62×2, Workspace7×2, lifecycle, VT, Session, Host, Codec, Wire, Provider and native app fixtures.
- Backend76867: Host60 active cases, public SDK, real SSH restart/sibling and prolonged outage/reroute all pass. The outage test observes at least five failed dials before recovery. Production backoff is1–30s, with four globally concurrent attempts, not four total retries.
- Native SSH clipboard90024: both ordinary and Fast targets pass (2.8/3.2s), including ordinary-input focus exclusion.
- Optimized full browser fixture38642: passes in102.5s on unchanged721MiB/no-swap Debian VM. Mac checks UUID CLI, Tab, RGB/OSC8, find, actual reload,12k output, idle30s park/wake, mirrors/control transfer and both-axis panning. Linux checks the same real GPUI WebGL renderer, local/SSH Tab/RGB/OSC8/find, physical clipboard and actual reload. Artifacts: `/private/tmp/c1-browser-opt12-full-pass`.
- Fixture30769 passes the shipped renderer flags in101.5s, including memory saver. Focused repeat71991 passes in117.0s with the Linux runner supplying its backend default; memory saver was disabled only in that repeat, having passed twice.
- Earlier debug Wasm46,596,431B repeatedly OOM-killed Chromium on reload. Explicit viewer teardown alone did not fix it. Same source built with `-c opt` is24,283,027B and passes. No VM resources/swap/timeouts were increased. This is a bounded workload result, not a universal memory ceiling or leak-free guarantee.
- App `permission_alerts_test`:3 behavior cases and Settings `preferences_test` pass. Native focused2 cases record43815/verify43372 pass: extra alerts and the other lane's D199 routing. Only the extra-alert behavior test belongs to this lane's publication; its reviewed image stays a chat artifact rather than a golden coupled to other dirty UI lanes. Final focused native alert behavior22783 passes in2.4s after removing pixel coupling. Do not scoop D199 source.
- HOOKS artifacts independently checked: hooks12, settingsagent_asks6, payloads13, CLIhooks6 pass. Actual all-vendor trust/startup certification remains separate work.
- Latest live optimized candidate63921 passes basic paint/discovery in12.5s without input injection, then optimized13 is promoted. Full input/reload acceptance uses disposable fixture shells, never the user's live shells. Recovery upgrade proof is private `recovery-upgrade-proof.json`; local41874/41875 and remote6203/6207 retain exact process start times.

## Live demo

- Private state: `/Users/avestabarzegar/.humata-terminal-demo-c1` (0700).
- Stable URL/capability is in private `viewer.url`; never copy its token into Git, diagnostics or this handoff.
- Promoted optimized release: `build-c991d30b565fd3ca0e30`, protocol3 (optimized13).
- Host was gracefully upgraded41873→91516→22003; read current `server.json` before any future signal. Never signal a historical PID without matching its command/state path.
- Original local holder41874 and Zsh41875 survived, including process start times. Original SSH holder6203 and current Bash6207 survived, including start times. SSH holder argv still records the original fixture Zsh launch; the actual child is Bash after the earlier explicit exec.
- Workspace migrated schema2→3, preserving routes1/2 and binding existing local/ssh startup profiles; no duplicate shell was launched. UUIDs are now durable.
- Private pre/post-shutdown rollback metadata/checkpoints: latest `rollback-recovery-20261116-095806`, plus earlier `rollback-v1-20261116-110312`, inside that state. Do not restore over a live owner. Older hosts cannot write schema3 safely; a rollback needs the compatible saved schema/checkpoints plus matching binary.
- Host/holders and VM remain running. Refresh an old browser page to load the new protocol3 viewer.
- Actual live SSH shell uses its own Bash configuration; no personal Mac dotfiles/history/keys were copied. Historical output still includes the earlier isolated prompt fixture.

## Frozen artifacts

- Host: `/private/tmp/c1-host-recovery-9708/terminal_host`, SHA256 `65464260898618b26d2dbcc0ae513edc181efd8b271ffaf627452c5e08b192b7`. Gate9708 is the final production source.
- Matching holder: `/Users/avestabarzegar/.humata-terminal-demo-c1/bin/holder-104078dd97a66a8af012`.
- Optimized viewer: `/private/tmp/c1-terminal-gpui-resize13-67244/bundle`, Wasm SHA256 `e8cb1c83f7d1ddec317d6860e76042e2d729e8f6c59705dfb1f394971545033c`. Source/manifest copies are adjacent. Opt13 passes the complete vertical-resize/browser fixture53988.
- Debug failure evidence: `/private/tmp/c1-browser11-explicit-cleanup-reload-failure`; do not confuse it with optimized acceptance. Kernel PID13781 ties the fourth OOM kill to that run. The shared event ring did not prove pagehide cleanup completed.
- User images are under `/Users/avestabarzegar/.codex/visualizations/2026/11/15/01a1dea3-36e0-71d1-a99e-717cc3bb9b22/`. `terminal-optimized-linux-ssh-verified.png` was sent to chat. `terminal-live-protocol3.png` captures the prior upgrade. New resize and live images are in `terminal-resilience-nov16/`.

## Build and fixture commands

Export `PATH=$PWD/.tools/bin:$PATH`. Warm bases:

- Runtime/backend: `--output_base=/private/tmp/c1-terminal-browser-runtime --symlink_prefix=/private/tmp/c1-browser-`.
- GUI: `--output_base=/private/tmp/c1-terminal-gpui-build --symlink_prefix=/private/tmp/c1-gpui- --jobs=2`.

Build release browser assets with `bazel build -c opt //apps/humata2/terminal/web:bundle`. The root `terminal-ssh-demo.sh` helper and `platform_test/SSH_DEMO.md` describe the dedicated no-mount Lima OpenSSH fixture. VM `h2-terminal-c1` is Debian13 arm64,1CPU/.75GiB/4GiB; keep it running. Do not change user SSH config, keys, shell files or VM resources to hide test failures.

The runtime fixture receives absolute Bazel-built Host/holder/bundle paths via `TERMINAL_TEST_HOST`, `TERMINAL_TEST_HOLDER`, `TERMINAL_TEST_BUNDLE`, plus trusted `TERMINAL_TEST_SSH_CONFIG`. Set `TERMINAL_TEST_CLI=1`, `TERMINAL_TEST_LINUX_PROOF=1`, and `TERMINAL_BROWSER_{TAB,STYLE,CLIPBOARD,MEMORY,MIRROR,PAN,RESIZE}=1` for full stress acceptance. Node executable: `/Users/avestabarzegar/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node`. Every fixture owns/stops its three shells and removes only its scratch directories.

Actual browser runtime tests need `--strategy=TestRunner=local`; keep compilation sandboxed. Mac Chrome gets disposable profiles, mock keychain and basic password storage. The first live candidate attempt omitted this strategy and failed before Page.enable; the corrected local execution passes. Never operate on the user's Chrome profile/keychain.

## Ownership and publication

- Baseline code commit4e20a245 and hash-fill ea510e43 publish the generic terminal experiment. The current recovery/resize/fork milestone extends D206 and has its own GAPS_REVIEW entry. Its final VM gate28349, reviewed Host72/realCodex gate9708 and live candidate63921 pass. All three helper lanes are complete; root owns selective publication and hash-fill. Further adoption boundaries below remain explicit.
- Every code commit needs same-commit `GAPS_REVIEW.md` and accurate C1 `OVERVIEW` block, then a hash-fill follow-up and authorized push to shared main; verify remote SHA. Commit prefix: `Humata 2 (minion C1): ...`. No attribution trailers.
- Preserve `.claude/**`, ui-prototype, daemon/proto and every unrelated lane. No broad `git add .`, reset or stash. Use `git -c lfs.storage=/tmp/humata2-minion-c-lfs` for PNG-aware Git operations; inherited LFS storage points outside this checkout.
- Owned pin artifacts: `/private/tmp/c1-owned-pin-artifacts/owned-native-pins.patch` and `owned-module.patch`. They exclude foreign OpenCode/Find/Marks/Mirror changes and unrelated MODULE moves. Exact native checksum `0022c54cf8b22f150c1b18287ab61cd4ff8a28b0af5c65dcfa10f39f85bd9aba` comes from Bazel's typed digest; mod tidy without repin validates it. External dependency contexts are unchanged.
- Source ownership inventories and post-format GPUI patch are beside those pin patches. HEAD already contains scrollbar implementation, so the owned patch does not scoop a foreign scrollbar change. Exclude engine/frame.rs and parity.rs formatting-only edits.
- Root shared-file candidates are in `/private/tmp/c1-owned-staging/files`; build them from HEAD plus only owned changes. In settings, only the extra-alert preference/action/reducer is ours; Row/palette, D199 and Codex-trust source belong to others.
- A private repin experiment accidentally generated snapshot/BUILD.bazel through a symlink at10:45:54UTC. Root restored it byte-for-byte from HEAD, retained the generated backup, and the final50-test suite passes afterward. Bounded audit found no other write-through. Never run a generator against a private projection containing writable source symlinks; final pins contain no Path dependency.

## Remaining adoption boundaries

- Main-daemon/prototype adoption, personal-account provider trust/configuration and full platform IME/accessibility/context-menu/font-fallback coverage remain separate work. Supplied structured provider events are tested; no transcript scraping or agent-completion guessing.
- No Mosh UDP roaming/predictive echo claim. Reachable networking, SSH trust/auth and a live compatible holder are required. Bounded replay gaps or ambiguous mechanical writes require explicit recovery; disk display state cannot resurrect arbitrary processes after reboot.
- Sustained throughput/latency under severe CPU/disk contention, ENOSPC/EIO/fsync fault matrix and long-duration retention remain stress work, not universal guarantees.
- Visible mirrors stay resident. Parking frees allocations for reuse but does not guarantee lower process RSS. Host history/checkpoint policy is independent and may contain recent sensitive output; private permissions and explicit off/bounds remain essential.
- Optional file/image transfer and OS notification policy belong to consumers. Inline terminal image protocols are not implemented by this text-grid renderer.
- Neovim-specific clipboard follow-up is lowest priority and explicitly deferred.
