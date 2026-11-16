# Humata 2 ideas

Ideas for after the app is done (V1). Nothing here is planned or signed off.
Each entry says what it is for, the owner's notes, and what is known so far.
When one is picked up, it becomes a milestone in `workflow/milestones/` and
its entry links there.

## Mounted machines (one file system in Finder)

- **For:** copying, syncing and moving files between machines from Finder or a
  shell, and letting local agents read remote files. Agents still run on the
  remote machine, which is faster and uses its resources; this is for files,
  not for running work.
- **Shape (owner, 2026-11-15):** `~/Humata/host/…` for the host's own files,
  `~/Humata/remote/<machine-name>/…` for each machine. Lazy: nothing is fetched
  until it is opened. Kept small and built on the daemon's existing file
  RPCs (FileService, transfers) rather than a general network file system.
- **Until then:** the in-app Files page (FILES) over FileService and
  transfers. Today it is two panes with a machine picker each, Copy and Move
  buttons, and drop in from Finder. The owner chose to keep it that simple
  (2026-11-15); a single tree across machines and drag between panes or out
  to Finder are ideas, not plans.
- **If picked up:** reserve `host` and `remote` under `~/Humata` as DK-2 does
  for `desk`, and share the layer with D63's read-only session-history file
  system.
- **Known so far:** macOS has no built-in FUSE, and macFUSE needs a kernel
  extension. The candidates are File Provider, FSKit and FUSE-T. Linux uses
  FUSE. Details in Research below.

### Research (advisor, 2026-11-15; checked on macOS 26.7, nothing built or mounted)

The best mechanism per OS for a lazy, RPC-backed, mostly read/copy mount:

- **Linux: FUSE through the `fuser` crate** (0.18.0, MIT). Without its `libfuse` feature it is pure Rust, so Bazel needs no system library. The machine needs `fusermount3` (the fuse3 package) to mount without root. It mounts on any folder the user owns, and each request carries the caller's pid.
- **macOS, simplest: the host daemon serves NFSv3 on loopback and `mount_nfs` mounts it.** No root, no extension, no entitlement, nothing to install, and it mounts at exactly `~/Humata/remote/<name>`. Crate: `nfs3_server` (0.11.1, BSD-3-Clause, tokio; its README says "incomplete but very functional"). Risks:
  - It listens on a loopback TCP port, against TR-5. 26.7's `mount_nfs` has a local-socket transport (`ticotsord`, seen in the binary); not tried.
  - A dead or slow daemon hangs Finder unless the mount is `soft` with a `deadtimeout`, and file handles must stay valid across a daemon restart.
  - Finder writes `.DS_Store` and `._*` files. The server must keep them on the host and never send them to a machine.
  - macOS asks once per app for "Network Volumes" access.
  - NFSv3 has no open or close and no locks, and tells the server only a uid, not which process is asking.
- **macOS, native: FSKit.** It does mount without a block device on macOS 26: 26.7 ships Apple's own `com.apple.fskit.ftp.appex` with `FSSupportsGenericURLResources` on and block resources off. It needs a sandboxed, signed app extension with the FSKit entitlement, the user switching it on in System Settings, and an App Group so the sandbox can reach humatad's socket. Rust: the `objc2-fs-kit` bindings (0.3.2), or a small Swift extension that forwards to Rust; rules_apple is already in MODULE.bazel. Risks: developer reports filed against macOS 27 betas list no byte-range locks, fsync never reaching URL-backed volumes, a rename-swap that destroys its target and stale entries after a root invalidation (one caching bug is fixed in 27.2), and Apple says the read/write path isn't optimized yet.
- **macOS, File Provider (replicated).** The best fit for Finder: a sidebar entry per machine, cloud badges, files fetched whole when opened, and whole-file uploads that map onto `ReadFile` and `Upload`. But it is a synced copy, not a mount: it lives under `~/Library/CloudStorage` (only a symlink can sit under `~/Humata`), fetched files stay on the Mac until the system evicts them (a recursive grep downloads all it touches), it needs a "changes since" feed FileService doesn't have, and it needs a sandboxed extension, an App Group and signing.
- **Not these:** macFUSE's kernel extension needs Reduced Security on Apple Silicon. Its FSKit backend (`-o backend=fskit`, macOS 26 for network volumes) and FUSE-T (NFS, SMB or FSKit underneath; free for non-commercial use only, bundling it in a commercial product needs a paid licence, and its libfuse is LGPL) are separate installs that put a C library over the same two system mechanisms. SMB and WebDAV have no mature Rust server.
- **Web:** a browser can't mount anything; the in-app Files page is the view there.

Limits on every mechanism:

- **Writes:** a main clone is read-only (FC-2). A move between machines arrives as a copy then a delete, so FM-14 holds only if a write is staged on the host and sent by the transfer engine once the file is whole, and a delete goes to the machine's trash. A copy cut off in Finder doesn't resume.
- **Freshness and speed:** remote changes raise no file events, and TR-7 forbids polling, so a listing refreshes when it is opened. An offline machine must answer with an error at once, never hang. It is not for git, builds or watchers.
- **Access:** the mount acts as the user (D144), so a local agent reading it skips its session's access rules. Expose only Humata's folders, never the home root. Only FUSE's pid lets the server tell callers apart.
- **Layout:** `host` and `remote` sit beside channel folders under the root, so reserve the names as DK-2 does `desk`. D63's open session-history file system should use the same layer.
- **In the app today (FILES):** two panes, each with a machine picker; Copy and Move buttons; drop in from Finder. It has no single tree of machines, no drag between panes and no drag out to Finder (the second half of FM-4), so the tree and drag in "Until then" above are not yet planned work.

**Recommendation:** one small Rust crate that turns FileService into file-system calls (stable ids from locations, a listing cache, reads in 1 MiB chunks on demand), with two thin fronts: `fuser` on Linux and loopback NFS on macOS. Start read-only (browse, open, copy out, local agents read). Then add copy-in through a host-side staging folder and the transfer engine, and delete to trash; nothing more. Spike first: NFS over a unix socket on 26.7, and how Finder behaves when a machine is offline. Move the macOS front to FSKit once its network path settles (recheck on macOS 27.x). Leave File Provider out unless Finder badges and offline copies become the goal.
