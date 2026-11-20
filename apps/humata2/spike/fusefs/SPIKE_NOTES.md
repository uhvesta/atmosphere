# FUSEFS `T1` Throwaway Spike Notes

> Scope: Gate `T1` validation probes for Humata 2 `0.6.0 FUSEFS` (`apps/humata2/spike/fusefs/`).
> Isolated throwaway evidence only — zero edits outside `apps/humata2/spike/fusefs/` and zero mutations to `~/.claude`, `~/.codex`, `~/.copilot`, or `~/.gemini`.

## Bazel targets to run

```bash
# Run all 4 automated T1 probe test suites:
bazel test //apps/humata2/spike/fusefs:all_tests

# Or run each probe individually:
bazel test //apps/humata2/spike/fusefs/fskit_probe:fskit_probe_test
bazel test //apps/humata2/spike/fusefs/cas_bench:cas_bench_test
bazel test //apps/humata2/spike/fusefs/sqlite_wal_probe:sqlite_wal_probe_test
bazel test //apps/humata2/spike/fusefs/posix_probe:posix_probe_test

# Run the BLAKE3 + FastCDC vs fixed-256K wire codec benchmark (release):
bazel run -c opt //apps/humata2/spike/fusefs/cas_bench:cas_bench

# Inspect / dry-run the macOS FSKit mount helper and local Unix bridge:
bazel run //apps/humata2/spike/fusefs/fskit_probe:mount_helper -- --dry-run
bazel run //apps/humata2/spike/fusefs/fskit_probe:mount_helper -- --probe-socket
```

---

## 1. macOS 26.7+ FSKit Signing, Entitlements & Mount Rules (`fskit_probe/`)

### Why `Humata 2.app` stays clean
- FSKit filesystem modules (`com.apple.fskit.fsmodule`) are App Extensions (`.appex`) embedded inside a host app's `Contents/Extensions/` directory.
- To avoid bloating `Humata 2.app` with filesystem IPC/cache code or risking `humatad2`/UI crashes when `fskitd` recycles the extension, the extension (`HumataFuseFSExtension.appex`) is kept as a tiny Swift/ObjC `FSUnaryFileSystem` shim that speaks a versioned length-prefixed protocol over an App Group Unix domain socket to the standalone `humata2_fusefs` process.

### Required `Info.plist` (`EXAppExtensionAttributes`)
- `EXExtensionPointIdentifier`: `com.apple.fskit.fsmodule`
- `EXPrincipalClass`: `$(PRODUCT_MODULE_NAME).HumataFuseFSExtension`
- `FSShortName`: `humatafs` (used with `/sbin/mount -F -t humatafs ...`)
- `FSSupportedPersonalities`: declares `FSGenericURLResource` support (e.g., `humata-fuse://main`) so `fskitd` activates the unary filesystem without requiring a block device (`FSBlockDeviceResource`).
- `FSMediaTypes`: empty/generic URL resource personality; no disk arbitration claims.

### Required Signing & Entitlements
- **Team & Developer ID:** Both the containing app (`Humata 2.app`) and `HumataFuseFSExtension.appex` must be signed with the same Apple Developer Team ID and notarized for distribution on macOS 26.7+. Ad-hoc signing is insufficient for `fskitd` module activation unless SIP/AMFI dev-mode overrides are active.
- **Entitlements (`HumataFuseFSExtension.entitlements`):**
  - `com.apple.security.app-sandbox = true` (mandatory for `com.apple.fskit.fsmodule` extensions).
  - `com.apple.security.application-groups = ["$(TeamIdentifierPrefix)com.humata2.fusefs"]` (grants the sandboxed `.appex` read/write access to the shared App Group container `~/Library/Group Containers/<TEAM>.com.humata2.fusefs/fusefs.sock` where `humata2_fusefs` listens).
  - No `com.apple.security.network.client` or broad filesystem entitlements are needed inside the `.appex` because all remote peer I/O and local CAS disk access happen in `humata2_fusefs` over the App Group Unix socket.
- **User activation:** macOS requires one-time approval in **System Settings › General › Login Items & Extensions › File System Extensions** (or `pluginkit -e use -i <bundle-id>` during local development). Until approved, Humata falls back cleanly to the native local symlink skeleton (`MountUnavailable` on remote opens).

### Mountpoint & Finder Visibility (`MNT_DONTBROWSE`)
- Command form:
  ```bash
  /sbin/mount -F -o nobrowse -t humatafs humata-fuse://main "$MOUNTPOINT"
  ```
- `-F` selects FSKit; `-o nobrowse` sets `MNT_DONTBROWSE` so Finder sidebars, Desktop, and Disk Utility do not show spurious ejectable volumes.
- **Direct `~/.humata/fuse/main` vs `/Volumes` + symlink:**
  - Mounting directly on a user-owned empty directory (`~/.humata/fuse/main`) with `-o nobrowse` keeps paths canonical without touching `/Volumes` (which requires elevated privileges or DiskArbitration ownership).
  - If an environment restricts non-`/Volumes` mountpoints, mounting at a hidden backing dir and symlinking `~/.humata/fuse/main` preserves canonical path resolution (`fs::canonicalize` must still be tested against agent tools).

### Caller Identity (`audit_token_t` vs `cap_<id>` Capability Roots)
- On the local Unix domain socket bridge, macOS provides the peer's exact PID (`SOL_LOCAL` / `LOCAL_PEERPID`) and full `audit_token_t` (`SOL_LOCAL` / `LOCAL_PEERTOKEN`, decoded via `audit_token_to_pid`, `audit_token_to_euid`, `audit_token_to_egid`).
- However, inside `fskitd` -> `FSVolume` callbacks, public FSKit request context only reliably exposes `uid`/`gid` (the XPC caller into the `.appex` is `fskitd`, not the originating user-space tool process).
- **Conclusion:** Per-caller PID is **not** reliably available across two same-UID processes through FSKit callbacks. Therefore, **`$HUMATA_FUSE_ROOT` capability-root paths (`cap_<id>`) are required** on macOS to distinguish sessions running under the same UID.

---

## 2. BLAKE3 + FastCDC Wire Benchmark (`cas_bench/`)
- Implemented with pure-Rust spec-compliant **BLAKE3** (verified against official BLAKE3 empty, `"abc"`, 1 KiB, and multi-chunk Merkle tree test vectors) + pure-Rust **FastCDC** (`min = 64 KiB`, `avg = 256 KiB`, `max = 1 MiB` Gear hash) + pinned `@crates//:lz4_flex` and `@crates//:zstd`.
- Benchmarks fixed `256 KiB` vs `FastCDC (64K/256K/1M)` across `none`, `lz4`, and `zstd` on a synthetic repo-like corpus (Rust source, Markdown prose, JSON transcripts, incompressible binary) and measures **boundary-shift deduplication** after a 64-byte insertion at offset `128 KiB`.
- Expected finding: fixed `256 KiB` invalidates every chunk after the insertion (0% tail dedup), whereas `FastCDC` re-synchronizes within 1–2 chunks and deduplicates >85% of the remaining file bytes.

---

## 3. SQLite WAL Matrix Probe (`sqlite_wal_probe/`)
- Validates `PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;` with concurrent readers/writers, `wal_checkpoint(TRUNCATE)`, and `PRAGMA integrity_check` on native tempdir disk.
- Registers a custom `sqlite3_vfs` wrapper via `libsqlite3-sys` to trace exact `xRead`/`xWrite`/`xSync`/`xLock`/`xShmMap`/`xShmLock` operations and simulate FUSE/FSKit without coherent shared-memory `-shm` mapping (`xShmMap` returning `SQLITE_IOERR_SHMMAP`).
- Confirms that SQLite WAL requires coherent `mmap`-backed `-shm` + advisory locks and **must remain strictly on native disk** (`<machine-root>/.sessions/<id>/config/<agent>/`), never behind FUSE/FSKit.

---

## 4. POSIX Atomic Write & Progress-Bounded `fsync` Probe (`posix_probe/`)
- Exercises `sed -i`-style temp-file + `fsync` + atomic `rename` + parent-dir `fsync`, verifying `expected_generation` CAS checks return `ESTALE` on concurrent modification.
- Exercises recursive `cp -r` under a turn-bounded target-directory subtree lease, verifying in-subtree operations succeed under a single approval while out-of-subtree writes fail with `EACCES`.
- Exercises progress-bounded `fsync`: active draining over a throttled link completes with `0` even when total wall time exceeds the stall timeout, whereas a stalled link with zero progress past the stall bound fails deterministically with `ENETUNREACH`/`EIO` (never `EAGAIN`).

---

## 5. Native Local Auth + OAuth Refresh Matrix

Verified from `apps/humata2/daemon/contain/src/{home,login,claude}.rs` and `apps/humata2/FUSEFS_SPEC.md` (zero reads/writes performed against live `~/.claude`, `~/.codex`, `~/.copilot`, or `~/.gemini`).

| Agent | Native per-session config env var | Real home state paths | Machine-local auth / credential paths (`Secret`, **excluded** from FUSE & sync) | Non-auth `[sync]` candidate files by category | OAuth refresh token writeback & macOS/Linux plumbing |
|---|---|---|---|---|---|
| **`claude`** (Claude Code) | `CLAUDE_CONFIG_DIR=<machine-root>/.sessions/<id>/config/claude` | `~/.claude/`, `~/.claude.json` | macOS: Login Keychain (`Claude Code-credentials`); Linux/file-backed: `~/.claude/.credentials.json`; Env: `CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_API_KEY`; inline MCP `env`/`headers` tokens | `skills`: `skills/`, `commands/`, `agents/`, `plugins/`<br>`instructions`: `CLAUDE.md`<br>`mcp`: scrubbed `.claude.json` `mcpServers` + `.mcp.json`<br>`settings`: `settings.json`, `settings.local.json`, `hooks/` | **macOS:** Set `CLAUDE_SECURESTORAGE_CONFIG_DIR=""` for verified versions (e.g. `2.1.325` per `contain/src/login.rs`) so Claude Code reads/refreshes the user's real Login Keychain entry directly without copying credentials.<br>**Linux:** Symlink `<session-config>/.credentials.json` -> `~/.claude/.credentials.json` (or atomic rename target) so OAuth refresh writes update the machine's real auth file in place. Keep `projects/` and `.claude.json` atomic temp-file renames on native disk. |
| **`codex`** (OpenAI Codex CLI) | `CODEX_HOME=<machine-root>/.sessions/<id>/config/codex` | `~/.codex/` | `~/.codex/auth.json` (file-backed OAuth/API tokens), OS keyring (macOS Keychain / Linux Secret Service if enabled), `OPENAI_API_KEY` | `skills`: `skills/`<br>`instructions`: `AGENTS.md`<br>`mcp`: scrubbed `[mcp_servers]` in `config.toml`<br>`settings`: non-secret keys in `config.toml` | Symlink `<session-config>/auth.json` -> `~/.codex/auth.json` on both macOS (when file-backed) and headless Linux so OAuth token refreshes persist in place to the machine's real login. Keep SQLite WAL state (`state_5.sqlite`, `thread_history_1.sqlite`) strictly native per-session. |
| **`copilot`** (GitHub Copilot CLI) | `COPILOT_HOME=<machine-root>/.sessions/<id>/config/copilot` | `~/.copilot/` | `~/.copilot/config.json` (when carrying OAuth/token fields), `~/.config/gh/hosts.yml`, OS Keychain / `GITHUB_TOKEN` / `GH_TOKEN` | `skills`: `skills/`, `agents/`<br>`instructions`: `copilot-instructions.md`<br>`mcp`: scrubbed `mcp-config.json`<br>`settings`: non-secret preferences split from auth | Keep machine-local GitHub auth (Keychain / `gh` auth / local auth file) wired natively to the machine's own login; scrub token fields before exposing `s/<id>/config/copilot` in FUSE. Keep `session-store.db` SQLite WAL strictly native. |
| **`agy`** (Antigravity CLI) | Native `~/.gemini/antigravity-cli` (or per-session composed config home when supported) | `~/.gemini/antigravity-cli/`, `~/.gemini/config/` | OAuth credentials / token caches under `~/.gemini/` (e.g. `oauth_creds.json`, `google_accounts.json`, Keychain entries), `GEMINI_API_KEY` / Google ADC | `skills`: `~/.gemini/antigravity-cli/skills/` (and `builtin/skills/` read-only)<br>`instructions`: `GEMINI.md` / rules<br>`mcp`: scrubbed `mcp_config.json`<br>`settings`: `settings.json`, `~/.gemini/config` | Never sync or expose OAuth/ADC credentials through FUSE (`CredentialOutsideMount`). Wire machine-local OAuth credential files via native local pass-through so token refresh updates the machine's own store without crossing peers. |

### Invariants across all 4 agents
1. **Never mutate real `~/.<agent>` during sync or rage-quit:** Host-synced non-auth layers live in `<machine-root>/.humata/synced-config/<gen>/` and are composed into `<machine-root>/.sessions/<id>/config/<agent>/`.
2. **Instant rage-quit (`sync.enabled = false` or per-category OFF):** Atomically re-points native symlinks and re-renders merged non-secret config files to source from the machine's own `~/.<agent>`, with zero holder restarts and zero credential loss.
3. **Zero secrets on the wire or in FUSE:** Every `Secret`-classified file returns `CredentialOutsideMount` (`EACCES`) through `s/<id>/config/<agent>` in FUSE and is excluded from `SessionSnapshot` and `ConfigBundle`.
4. **OAuth refresh atomicity:** Agents that refresh OAuth tokens via `write(tmp) -> fsync -> rename(tmp, auth_file)` inside a symlinked file's directory will replace the symlink if the parent dir is the session dir rather than the real auth dir! Therefore, `T1` flags an important implementation rule for `humata2_config_compose`:
   - For CLIs that rewrite `auth.json` / `.credentials.json` via atomic temp-file + `rename`, either verify in-place truncation vs rename behavior per CLI version, or watch/propagate refreshed auth back (or use the CLI's native keyring / dedicated auth env var where supported) so a `rename` inside `<session-config>` never silently detaches the symlink from `~/.<agent>/auth.json`.
