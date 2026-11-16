# Agent guidelines

## Builds

- Build, test, bench and lint only through Bazel. Never run `cargo` directly; if something needs it, add a Bazel target instead.

## Errors

- Return explicit error types, not a catch-all like `Box<dyn Error>` or `anyhow::Error`. Each variant records what went wrong in its fields and keeps the underlying error as its source.
- Write user-facing messages for the person who has to fix the problem: say what failed and how to resolve it. When the obvious fix is unsafe or incomplete, say so. For example, a hash mismatch must not just say "re-pin", and a rate limit must say to wait, not only to add a token.

## API surface

- Keep visibility minimal. Bazel targets get the narrowest `visibility` that works, and Rust items stay private unless another crate uses them.
- Add a public function, type or target only when something outside its package needs it.

## Tests

- Test that code behaves as expected in isolation: given these inputs, it produces this result or this error.
- Don't write tests that restate the implementation, such as asserting on the exact wording of a message or that a function calls another one. They break when the code changes, not when behavior does.

## Terminal experiment

- The local SSH test server is OpenSSH inside the dedicated, no-mount `h2-terminal-c1` Lima VM. Use the root `terminal-ssh-demo.sh` helper and follow [SSH demo setup](apps/humata2/terminal/platform_test/SSH_DEMO.md). It is a real remote shell, not simulated terminal output.
- Desktop and browser mirror the same local terminal host, including its SSH-backed sessions. Preserve live holders when refreshing viewers or upgrading that host. The [terminal handoff](apps/humata2/terminal/lab/HANDOFF.md) records current verification and continuation state.
