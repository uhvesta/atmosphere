//! CLI mount helper for Gate T1 macOS 26.7+ FSKit probe.
//!
//! Supports:
//! - `--dry-run`: prints the exact `/sbin/mount -F -o nobrowse -t humatafs` command.
//! - `--probe-socket`: spins up an ephemeral Unix socket bridge and verifies
//!   `audit_token_t` / `LOCAL_PEERPID` caller-PID extraction.
//! - `--mount <resource-url> <mountpoint>`: executes `/sbin/mount -F -o nobrowse`.

use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::Command;
use std::thread;

use fusefs_fskit_probe::{
    BridgeRequest, MountCommandSpec, client_round_trip, serve_one_request,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--dry-run") | None => {
            let mountpoint = PathBuf::from(
                args.get(1)
                    .cloned()
                    .unwrap_or_else(|| "~/.humata/fuse/main".to_owned()),
            );
            let spec = MountCommandSpec::for_fskit("humata-fuse://main", &mountpoint)
                .expect("build mount spec");
            println!(
                "dry-run: {} {}",
                spec.program.display(),
                spec.args.join(" ")
            );
        }
        Some("--probe-socket") => {
            let dir = tempfile::tempdir().expect("tempdir");
            let sock = dir.path().join("fusefs.sock");
            let listener = UnixListener::bind(&sock).expect("bind socket");
            let handle = thread::spawn(move || serve_one_request(&listener));
            let resp = client_round_trip(&sock, &BridgeRequest::Ping).expect("ping");
            let peer = handle.join().expect("join").expect("serve");
            println!("bridge ok: peer={peer:?} resp={resp:?}");
        }
        Some("--mount") => {
            let url = args
                .get(1)
                .expect("usage: mount_helper --mount <humata-fuse://...> <mountpoint>");
            let mountpoint = PathBuf::from(
                args.get(2)
                    .expect("usage: mount_helper --mount <humata-fuse://...> <mountpoint>"),
            );
            let spec = MountCommandSpec::for_fskit(url, &mountpoint).expect("build mount spec");
            let status = Command::new(&spec.program)
                .args(&spec.args)
                .status()
                .expect("spawn /sbin/mount");
            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        Some(other) => {
            eprintln!("unknown flag: {other}");
            eprintln!("usage: mount_helper [--dry-run [mountpoint] | --probe-socket | --mount <url> <mountpoint>]");
            std::process::exit(2);
        }
    }
}
