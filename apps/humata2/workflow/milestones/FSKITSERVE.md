# FSKITSERVE — on a Mac, Humata's files really mount through FSKit

Split out of FUSEFSWIRE on 2026-11-20 (D911). FUSEFSWIRE found that the helper `humata2_fusefs serve` serves no volume on either OS. Linux was fixed in FUSEFSWIRE with fusermount3, the wire codec and the serve loop. The Mac needs the FSKit side, which is this milestone.

## Outcome
On a Mac, Humata's files really mount through FSKit, and the files tools are only the fallback.

## Leads (FUSEFSWIRE manager, 2026-11-20; unverified)
- `daemon/fusefs_adapter/src/main.rs` `run_serve`:
  - on macOS it builds `PosixMountBackend` (`libs/fusefs/host/src/supervisor.rs`), which runs `/sbin/mount -F -t fskit fusefs://fskit/humata2 <mnt>`;
  - it never starts `FsKitBridgeServer`, so nothing serves `HumataVolumeHost` to an extension.
- `libs/fusefs/fskit`:
  - `bridge.rs` (`FsKitBridgeServer`, `FsKitBridgeClient`) and `adapter.rs` (`FsKitAdapter`, with `cap_<id>` routing through `CapabilityRouter`) are written;
  - they are unit-tested only.
- **The appex bundle:** an FSKit file-system extension (`.appex`) has to ship inside Humata.app. Look at how `app/app.bzl` assembles the bundle, and where an extension target and its Info.plist (`EXAppExtensionAttributes`, the FSKit module identity) go.
- **Entitlements and signing:**
  - the FSKit extension entitlement;
  - the App Group the bridge socket lives in;
  - signing the appex with the existing Humata signing credentials.

  The owner turns the extension on in System Settings, once.
- `humatad/src/fusefs.rs` already reports CannotSetUp with a reason when the mount is refused, so agents get the files tools. Keep that path as the fallback.

## Exit
- On a Mac with the extension on:
  - turning FUSE on mounts the tree at `$HUMATA_FUSE_ROOT`;
  - an agent reads and writes its session's files through the mount;
  - a killed helper comes back once (the FUSEFSWIRE T13 case-5 shape, run on the Mac).
- With the extension off or unavailable, Settings reads "can't set up" with a reason that says how to turn it on. Agents are told to use the files tools.
- No GUI windows in tests. Headless checks plus one live run on the owner's Mac when told the screen is free.

## Ships
0.10.5, before WEBTODO.
