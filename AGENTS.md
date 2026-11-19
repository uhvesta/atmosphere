# Agent guidelines

## Builds

- Build, test, bench and lint only through Bazel. Never run `cargo` directly; if something needs it, add a Bazel target instead.
- For fast local edit-test loops, pass `--config=fast` to skip lint aspects (`clippy`, etc.) so Rust crates compile once instead of twice. Always run without `--config=fast` (default `--config=lint`) before committing.

## Errors

- Return explicit error types, not a catch-all like `Box<dyn Error>` or `anyhow::Error`. Each variant records what went wrong in its fields and keeps the underlying error as its source.
- Write user-facing messages for the person who has to fix the problem: say what failed and how to resolve it. When the obvious fix is unsafe or incomplete, say so. For example, a hash mismatch must not just say "re-pin", and a rate limit must say to wait, not only to add a token.

## API surface

- Keep visibility minimal. Bazel targets get the narrowest `visibility` that works, and Rust items stay private unless another crate uses them.
- Add a public function, type or target only when something outside its package needs it.

## Tests

- Test that code behaves as expected in isolation: given these inputs, it produces this result or this error.
- Don't write tests that restate the implementation, such as asserting on the exact wording of a message or that a function calls another one. They break when the code changes, not when behavior does.

## Humata 2 crate & test cache boundaries

Keep Bazel build and test cache isolation intact when adding code or tests under `apps/humata2/`:

- **Shared daemon test harness (`:test_common`):** Never add `mod common;` or `#[path = "common/..."]` to `apps/humata2/daemon/humatad/tests/*.rs`. Import from the shared library (`use humatad_test_common as common;` / `use humatad_test_common::<mod>;`) and depend on `":test_common"` so the ~275 KB harness compiles once instead of 216×.
- **Scoped daemon test binaries (Tier B vs Tier C):** Route every new `daemon/humatad` or `clients/live_*` integration test onto the narrowest scoped profile that covers its actual runtime calls (`humatad2_test_power_cut`, `humatad2_test_machines`, `humatad2_test_review_remote`, `humatad2_test_catalog_remote`, `humatad2_test_away_board`, `humatad2_test_transfers`, plus `_release`/`_faults` twins only when needed). See [SPLIT_CAPABILITY_MATRIX.md](apps/humata2/workflow/milestones/SPLIT_CAPABILITY_MATRIX.md). Reserve `:humatad2` / `:humatad2_admin` for explicit Tier-C full-daemon E2E tests (auth/CLI/MCP/full-stack upgrades).
- **Prefer in-process unit tests:** Put single-crate logic, validation, and state tests in `rust_test` targets on the leaf crate (`daemon/svc_*`, `daemon/sessions_*`, `ui/*`, `features/*`) instead of spawning a daemon subprocess or mounting full `humata2_ui::Shell`.
- **Narrow dependencies only:**
  - `features/*`, `ui/*`, and `clients/live_*` must depend on `//apps/humata2/clients:api`, never the monolithic `//apps/humata2/clients` crate (which holds fakes/impls).
  - Narrow daemon and client crates must depend on `//apps/humata2/api/grpc:<domain>` (or `//apps/humata2/api/proto:<domain>_rust`), never the monolithic `//apps/humata2/daemon/humatad:grpc` umbrella unless they genuinely span all services.
  - Keep `apps/humata2/daemon/ports` limited to stable IDs, value types, and narrow traits; do not pull service implementations or the full gRPC umbrella into `ports`, `rpc_kit`, or `runtime`.
- **Multi-binary builds:** When building several large daemon binaries at once locally, pass `--jobs=4` or `--jobs=6` to avoid `rustc` + `clippy` CPU thrashing.
- **Keep test runtimes fast (no hidden waits):**
  - Never add fixed `sleep` waits or slow poll loops (`>= 20ms`) in tests or harnesses; wait on events/channels or use tight polls (`2ms`) with generous deadlines.
  - Fake agent scripts in test harnesses must set `stty -icanon 2>/dev/null` so holders do not wait out the 5 s canonical-mode keys limit when typing.
  - Guard teardown `pkill -f` behind checking whether the test's `holders/` directory actually has entries, and pass short admin-only retry knobs (`HUMATA2_PEER_RETRY_MS=20`) in multi-machine test rigs.

## Terminal experiment

- The local SSH test server is OpenSSH inside the dedicated, no-mount `h2-terminal-c1` Lima VM. Use the root `terminal-ssh-demo.sh` helper and follow [SSH demo setup](apps/humata2/terminal/platform_test/SSH_DEMO.md). It is a real remote shell, not simulated terminal output.
- Desktop and browser mirror the same local terminal host, including its SSH-backed sessions. Preserve live holders when refreshing viewers or upgrading that host. The [terminal handoff](apps/humata2/terminal/lab/HANDOFF.md) records current verification and continuation state.
