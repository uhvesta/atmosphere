//! How long an answer takes over the stdio peer link (TR-10), measured on
//! a real `humatad2 serve --stdio` child, as the Mac reaches a machine
//! over SSH but without the network. Prints percentiles; asserts nothing.
//!
//! The link serves only unary calls so far (Uplink, the event stream, is
//! not built), so this measures a WhoAmI round trip: the floor that
//! event delivery over the link sits on. Run it on a release build:
//!
//!     bazel test -c opt //apps/humata2/daemon/humatad:peer_link_latency_bench \
//!         --test_output=streamed --test_arg=--nocapture

use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use humata2_grpc::humata::v1 as pb;
use hyper_util::rt::TokioIo;
use tokio::process::Command;
use tonic::transport::{Endpoint, Uri};

const WARMUP: usize = 200;
const SAMPLES: usize = 5000;

#[tokio::test(flavor = "multi_thread")]
async fn peer_link_round_trip() {
    let binary = std::fs::canonicalize(PathBuf::from(
        std::env::var_os("HUMATAD2").expect("HUMATAD2 names the daemon"),
    ))
    .expect("the daemon binary exists");
    let dir = tempfile::tempdir().expect("a temporary folder");
    let state = dir.path().join("state");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&state)
        .expect("the state folder");
    let mut child = Command::new(binary)
        .args(["serve", "--stdio", "--state"])
        .arg(&state)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("serve --stdio starts");
    let stdio = tokio::io::join(
        child.stdout.take().expect("piped stdout"),
        child.stdin.take().expect("piped stdin"),
    );
    let stdio = Arc::new(Mutex::new(Some(stdio)));
    let channel = Endpoint::from_static("http://link")
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let stdio = stdio.lock().ok().and_then(|mut stdio| stdio.take());
            async move {
                stdio
                    .map(TokioIo::new)
                    .ok_or_else(|| std::io::Error::other("a stdio link carries one connection"))
            }
        }))
        .await
        .expect("the link connects");
    let mut daemon = pb::daemon_service_client::DaemonServiceClient::new(channel);

    let mut took = Vec::with_capacity(SAMPLES);
    for sample in 0..WARMUP + SAMPLES {
        let started = Instant::now();
        daemon
            .who_am_i(pb::WhoAmIRequest {})
            .await
            .expect("WhoAmI answers on the link");
        if sample >= WARMUP {
            took.push(started.elapsed());
        }
    }
    took.sort_unstable();
    let at = |fraction: f64| took[((took.len() - 1) as f64 * fraction) as usize];
    let micros = |duration: Duration| duration.as_secs_f64() * 1e6;
    println!(
        "peer link WhoAmI round trip over {SAMPLES} calls: p50 {:.0} µs, p90 {:.0} µs, p99 {:.0} µs, max {:.0} µs",
        micros(at(0.5)),
        micros(at(0.9)),
        micros(at(0.99)),
        micros(at(1.0)),
    );
}
