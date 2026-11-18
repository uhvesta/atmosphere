//! What one holder costs the machine (FAST): its memory and CPU while idle
//! with nothing printed yet, and after its program printed three times the
//! 8 MiB scrollback humatad2 gives every tab, then went quiet. Measured
//! on a real `humata2_holder` with no client attached, as a tab in the
//! background is. Prints numbers; asserts nothing. Run it on a release
//! build:
//!
//!     bazel test -c opt //apps/humata2/daemon/holder:cost_bench \
//!         --test_output=streamed --test_arg=--nocapture

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use humata2_holder::{HolderSpec, spawn};
use rustix::process::{Pid, Signal, kill_process};

/// humatad2's `--scrollback-bytes` default.
const SCROLLBACK: usize = 8 << 20;
/// How long the idle CPU is measured for.
const IDLE: Duration = Duration::from_secs(10);

fn holder_binary() -> PathBuf {
    let path = std::env::var_os("HUMATA2_HOLDER")
        .map(PathBuf::from)
        .expect("set HUMATA2_HOLDER to the humata2_holder binary");
    std::fs::canonicalize(path).expect("the holder binary exists")
}

/// Resident memory in KiB and CPU time in milliseconds, from `ps`.
fn usage(pid: u32) -> (u64, u64) {
    let out = Command::new("ps")
        .args(["-o", "rss=,time=", "-p", &pid.to_string()])
        .output()
        .expect("ps runs");
    let text = String::from_utf8_lossy(&out.stdout);
    let mut fields = text.split_whitespace();
    let rss = fields.next().and_then(|rss| rss.parse().ok()).expect("an rss");
    let time = fields.next().expect("a cpu time");
    // [[hh:]mm:]ss.cc
    let mut ms = 0f64;
    for part in time.split(':') {
        ms = ms * 60.0 + part.parse::<f64>().expect("a time part");
    }
    (rss, (ms * 1000.0).round() as u64)
}

struct Running(u32);

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(pid) = i32::try_from(self.0).ok().and_then(Pid::from_raw) {
            let _ = kill_process(pid, Signal::TERM);
        }
    }
}

fn start(script: &str) -> (Running, tempfile::TempDir) {
    let dir = tempfile::Builder::new().prefix("h2c").tempdir_in("/tmp").expect("a folder");
    let spec = HolderSpec {
        holder: holder_binary(),
        socket: dir.path().join("t.sock"),
        program: PathBuf::from("/bin/sh"),
        args: vec!["-c".into(), script.into()],
        cwd: dir.path().to_owned(),
        env: Vec::new(),
        cols: 200,
        rows: 60,
        scrollback_bytes: SCROLLBACK,
        log: dir.path().join("holder.log"),
    };
    let spawned = spawn(&spec).expect("the holder starts");
    (Running(spawned.pid), dir)
}

/// Waits until the holder's CPU time stops moving for two seconds.
fn settle(pid: u32) {
    let mut last = usage(pid).1;
    let mut still = 0;
    while still < 4 {
        std::thread::sleep(Duration::from_millis(500));
        let now = usage(pid).1;
        still = if now == last { still + 1 } else { 0 };
        last = now;
    }
}

fn measure(what: &str, script: &str) {
    let (holder, _dir) = start(script);
    settle(holder.0);
    let (rss, before) = usage(holder.0);
    std::thread::sleep(IDLE);
    let (_, after) = usage(holder.0);
    // Straight to stdout, not `println!`, so the numbers reach the test
    // log even when the harness captures output.
    let _ = writeln!(
        std::io::stdout(),
        "holder, {what}: resident {:.1} MiB, CPU {} ms over {} s idle",
        rss as f64 / 1024.0,
        after - before,
        IDLE.as_secs()
    );
}

#[test]
fn holder_cost() {
    measure("program idle, nothing printed", "exec sleep 3600");
    measure(
        "after 24 MiB of output (3x its 8 MiB scrollback), then idle",
        "yes 0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz | head -c 25165824; exec sleep 3600",
    );
}
