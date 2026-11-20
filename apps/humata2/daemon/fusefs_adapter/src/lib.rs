//! Humata 2 FUSEFS adapter (`//apps/humata2/daemon/fusefs_adapter`).
//!
//! Wires the generic `//libs/fusefs/*` crates into Humata 2's virtual tree,
//! peer CAS transport, access/lease/MASSDELETE policy, and materialization CLI:
//! - [`layout`] (T6): virtual tree projector (`s/`, `m/`, `channels/`, `desks/`,
//!   `machines/`, `requests/`, `control/`, `context/`, `.humata-omitted.json`),
//!   stable-by-id `.sessions/<id>/` placement, and daemon reconnect survival.
//! - [`transport`] (T7): [`PeerChunkTransport`] with `AWAY` autonomy, coalescing
//!   fire-and-forget [`InvalidationRing`], host-relayed routing, and Interactive
//!   vs. Bulk QoS prioritization.
//! - [`access`] (T9 & T10): [`HumataAccessHook`] with subtree `WriteLease`s,
//!   `< 50 ms` `EACCES` + `requests/<id>.json`, `ESTALE` generation checks,
//!   local [`AwayJournal`], and `MASSDELETE` `EPERM` root protection.
//! - [`materialize`] (T10): `fs status` summary and `fs materialize` byte-count
//!   confirmation planner.

pub mod access;
pub mod layout;
#[cfg(target_os = "linux")]
pub mod linux_mount;
pub mod materialize;
pub mod transport;

pub use access::{
    AwayJournal, AwayJournalEntry, HumataAccessHook, MAX_TURN_LEASE_DURATION_MS,
    ProtectedRootPolicy, WriteAskDescriptor,
};
pub use layout::{
    BuildCacheStub, DEFAULT_BUILD_CACHE_DIRS, DaemonLinkState, HUMATA_OMITTED_STUB_FILENAME,
    HumataTopology, HumataTreeProjector, HumataVolumeHost, MachineProjection, SessionFileSpec,
    SessionHolderRecord, SessionMetaJson, SessionProjection, DeskProjection,
    is_default_build_cache_dir, read_all_bytes, session_canonical_root, stable_session_native_path,
};
pub use materialize::{
    DEFAULT_CONFIRMATION_THRESHOLD_BYTES, FsMountMode, FsStatusSummary, MaterializeError,
    MaterializePlan, MaterializePlanner, MaterializeReport, MaterializeTarget, format_bytes,
};
pub use transport::{
    InvalidationEvent, InvalidationRing, InvalidationRingStats, PeerChunkTransport,
    PeerTransportStats, QosGate, QosPermit, QosStats, RouteTarget, RoutingTable,
};

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    use fusefs_cas::{ChunkTransport, LoopbackTransport, TransportError, TransportQoS};
    use fusefs_host::MountState;
    use fusefs_vfs::{
        CallerContext, FileKind, LeaseTable, OpenFlags, ROOT_NODE_ID, VfsError, VirtualPath,
        Volume,
    };

    use super::*;

    fn sample_topology(lazy_hits: Arc<AtomicUsize>) -> HumataTopology {
        let loader_hits = Arc::clone(&lazy_hits);
        let lazy_loader: fusefs_vfs::ChunkLoader = Arc::new(move |offset, size| {
            loader_hits.fetch_add(1, Ordering::SeqCst);
            let mut buf = vec![b'Z'; size as usize];
            if offset == 0 && !buf.is_empty() {
                buf[0] = b'H';
            }
            Ok(buf)
        });

        let sess_a = SessionProjection::new("sess-a", "mac-local", "/Users/dev/humata", 4242)
            .with_channel("eng-main")
            .with_desk("desk-1")
            .with_inline_file("src/lib.rs", b"pub fn hello() {}\n".to_vec())
            .with_inline_file("src/temp.rs", b"initial\n".to_vec())
            .with_lazy_file("assets/big.bin", 64 * 1024 * 1024, lazy_loader)
            .with_build_cache_stub(BuildCacheStub::build_cache(
                "target",
                2_147_483_648,
                18_400,
            ))
            .with_build_cache_stub(BuildCacheStub::build_cache(
                "node_modules",
                524_288_000,
                42_000,
            ))
            .with_build_cache_stub(BuildCacheStub::build_cache(
                ".git/objects",
                128_000_000,
                9_500,
            ))
            .with_context_file("notes.md", b"local authority context\n".to_vec());

        let sess_b = SessionProjection::new("sess-b", "linux-worker-1", "/srv/humata", 9001)
            .with_channel("eng-worker")
            .with_inline_file("README.md", b"remote worker repo\n".to_vec());

        HumataTopology {
            local_machine_id: "mac-local".to_string(),
            local_machine_root: PathBuf::from("/Users/dev/humata"),
            machines: vec![
                MachineProjection {
                    machine_id: "mac-local".to_string(),
                    display_name: "Mac Studio".to_string(),
                    online: true,
                    machine_root: PathBuf::from("/Users/dev/humata"),
                },
                MachineProjection {
                    machine_id: "linux-worker-1".to_string(),
                    display_name: "Worker 1".to_string(),
                    online: true,
                    machine_root: PathBuf::from("/srv/humata"),
                },
            ],
            sessions: vec![sess_a, sess_b],
            desks: vec![DeskProjection {
                desk_id: "desk-1".to_string(),
                session_ids: vec!["sess-a".to_string()],
            }],
            local_context_files: vec![("global.md".to_string(), b"local machine notes\n".to_vec())],
            control_status_json: None,
        }
    }

    // -------------------------------------------------------------------------
    // T6: Tree projection & zero-data metadata ops
    // -------------------------------------------------------------------------

    #[test]
    fn tree_projection_and_zero_data_metadata_ops_never_download_chunks() {
        let lazy_hits = Arc::new(AtomicUsize::new(0));
        let topo = sample_topology(Arc::clone(&lazy_hits));
        let leases = Arc::new(LeaseTable::new());
        let caller = CallerContext::new(500, 501, 20);
        // Grant lease inside /s/sess-a/repo so we can test rename & copy_file_range.
        leases.grant(
            500,
            VirtualPath::parse("/s/sess-a/repo").unwrap(),
            100_000,
            Some("turn-meta".into()),
        );

        let vol = HumataTreeProjector::project(&topo, Some(leases), None, None).unwrap();

        // Verify top-level namespaces exist.
        let root_entries = vol.readdir(&caller, ROOT_NODE_ID, 0).unwrap();
        let names: Vec<&str> = root_entries.iter().map(|e| e.name.as_str()).collect();
        for expected in [
            "s", "m", "channels", "desks", "machines", "requests", "control", "context",
        ] {
            assert!(names.contains(&expected), "missing top-level dir {expected}");
        }

        // Verify aliases point to canonical /s/<id>.
        let ch_link = vol.resolve_path(&caller, "/channels/eng-main/sess-a").unwrap();
        assert_eq!(ch_link.kind, FileKind::Symlink);
        assert_eq!(vol.readlink(&caller, ch_link.ino).unwrap(), "/s/sess-a");

        let m_link = vol.resolve_path(&caller, "/m/mac-local/s/sess-a").unwrap();
        assert_eq!(vol.readlink(&caller, m_link.ino).unwrap(), "/s/sess-a");

        let desk_link = vol.resolve_path(&caller, "/desks/desk-1/sess-a").unwrap();
        assert_eq!(vol.readlink(&caller, desk_link.ino).unwrap(), "/s/sess-a");

        // Verify stable-by-id native placement in meta.json.
        let meta_attr = vol.resolve_path(&caller, "/s/sess-a/meta.json").unwrap();
        let meta: SessionMetaJson =
            serde_json::from_slice(&read_all_bytes(&vol, &caller, &meta_attr).unwrap()).unwrap();
        assert_eq!(meta.holder_pid, 4242);
        assert_eq!(meta.native_path, "/Users/dev/humata/.sessions/sess-a");

        // Verify .humata-omitted.json stubs for target/, node_modules/, .git/objects/.
        for cache_dir in DEFAULT_BUILD_CACHE_DIRS {
            let stub_path = format!("/s/sess-a/repo/{cache_dir}/{HUMATA_OMITTED_STUB_FILENAME}");
            let attr = vol.resolve_path(&caller, &stub_path).unwrap();
            let raw = read_all_bytes(&vol, &caller, &attr).unwrap();
            let stub = BuildCacheStub::from_json_bytes(&raw).unwrap();
            assert_eq!(stub.reason, "build_cache");
            assert!(stub.original_bytes > 0);
        }

        // Zero-data metadata ops on lazy file: stat, readdir, rename, copy_file_range.
        let assets_dir = vol.resolve_path(&caller, "/s/sess-a/repo/assets").unwrap();
        let _entries = vol.readdir(&caller, assets_dir.ino, 0).unwrap();
        let lazy_attr = vol.resolve_path(&caller, "/s/sess-a/repo/assets/big.bin").unwrap();
        assert_eq!(lazy_attr.size, 64 * 1024 * 1024);

        // Rename lazy file -> still 0 chunk downloads.
        vol.rename(&caller, assets_dir.ino, "big.bin", assets_dir.ino, "big-renamed.bin")
            .unwrap();
        let renamed = vol
            .resolve_path(&caller, "/s/sess-a/repo/assets/big-renamed.bin")
            .unwrap();

        // copy_file_range from lazy file into another file -> still 0 chunk downloads!
        let dst = vol.resolve_path(&caller, "/s/sess-a/repo/src/temp.rs").unwrap();
        let copied = vol
            .copy_file_range(&caller, renamed.ino, 0, dst.ino, 0, 4096)
            .unwrap();
        assert_eq!(copied, 4096);
        assert_eq!(
            lazy_hits.load(Ordering::SeqCst),
            0,
            "metadata ops (stat/readdir/rename/copy_file_range) must NEVER download chunks"
        );

        // Only an explicit read invokes the chunk loader.
        let fh = vol.open(&caller, renamed.ino, OpenFlags::read_only()).unwrap();
        let data = vol.read(&caller, renamed.ino, fh, 0, 16).unwrap();
        assert_eq!(data.len(), 16);
        assert_eq!(data[0], b'H');
        assert_eq!(lazy_hits.load(Ordering::SeqCst), 1);
    }

    // -------------------------------------------------------------------------
    // T7: AWAY autonomy, coalescing invalidation ring, routing, & QoS
    // -------------------------------------------------------------------------

    #[test]
    fn away_autonomy_zero_ms_skip_fast_fail_invalidation_ring_and_routing() {
        let backend = Arc::new(LoopbackTransport::new());
        let manifest = backend.seed_file("/s/sess-a/repo/src/lib.rs", b"fn main() {}\n");
        let chunk_hash = manifest.chunks[0].hash;

        let host_reachable = Arc::new(AtomicBool::new(false));
        let routing = Arc::new(RoutingTable::new());
        routing.set_route(
            "linux-worker-2",
            RouteTarget::ViaHost {
                host: "mac-host".to_string(),
                target: "linux-worker-2".to_string(),
            },
        );
        routing.set_route(
            "linux-worker-1",
            RouteTarget::Direct("linux-worker-1".to_string()),
        );

        let ring = Arc::new(InvalidationRing::new(4));
        let transport = PeerChunkTransport::new(
            "linux-worker-1",
            "linux-worker-2",
            Arc::clone(&host_reachable),
            Arc::clone(&routing),
            Arc::clone(&ring),
            backend.clone(),
        );
        transport.seed_local_manifest("/s/sess-a/repo/src/lib.rs", manifest.clone());

        // Verify host-relayed routing table resolution.
        assert_eq!(
            transport.active_route(),
            RouteTarget::ViaHost {
                host: "mac-host".to_string(),
                target: "linux-worker-2".to_string(),
            }
        );
        assert!(transport.active_route().requires_host());

        // 1. AWAY 0ms revalidation skip when host_reachable == false.
        let t0 = Instant::now();
        let revalidated = transport
            .revalidate_local_path("/s/sess-a/repo/src/lib.rs")
            .unwrap()
            .unwrap();
        let reval_elapsed = t0.elapsed();
        assert_eq!(revalidated.generation, manifest.generation);
        assert!(
            reval_elapsed < Duration::from_millis(5),
            "AWAY revalidation skip took {reval_elapsed:?}"
        );
        assert_eq!(transport.stats().remote_round_trips, 0);
        assert_eq!(transport.stats().away_revalidation_skips, 1);

        // 2. <1ms offline peer fast-fail with NetworkUnreachable.
        let t1 = Instant::now();
        let err = transport
            .fetch_chunks(&[chunk_hash], TransportQoS::Interactive)
            .unwrap_err();
        let fail_elapsed = t1.elapsed();
        assert!(matches!(err, TransportError::NetworkUnreachable(_)));
        assert!(
            fail_elapsed < Duration::from_millis(2),
            "offline peer fast-fail took {fail_elapsed:?}"
        );
        assert_eq!(transport.stats().offline_fast_fails, 1);
        assert_eq!(transport.stats().remote_round_trips, 0);

        // 3. Coalescing fire-and-forget invalidation ring never blocks and bounds memory.
        for g in 1..=10u64 {
            ring.push(InvalidationEvent {
                path: "/s/sess-a/repo/hot.rs".to_string(),
                generation: g,
                machine_id: "linux-worker-1".to_string(),
            });
        }
        for i in 0..8 {
            ring.push(InvalidationEvent {
                path: format!("/s/sess-a/repo/file-{i}.rs"),
                generation: 1,
                machine_id: "linux-worker-1".to_string(),
            });
        }
        let ring_stats = ring.stats();
        assert_eq!(ring_stats.capacity, 4);
        assert_eq!(ring_stats.queued, 4);
        assert_eq!(ring_stats.coalesced_total, 9);
        assert!(ring_stats.dropped_total > 0);

        // 4. Online commit succeeds and enqueues an invalidation event.
        transport.set_host_reachable(true);
        let fetched = transport
            .fetch_chunks(&[chunk_hash], TransportQoS::Interactive)
            .unwrap();
        assert_eq!(fetched.len(), 1);
        let _ = ring.drain(10);
        let commit = transport
            .commit_file("/s/sess-a/repo/src/lib.rs", 1, manifest)
            .unwrap();
        assert_eq!(commit.generation, 2);
        let drained = ring.drain(10);
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].path, "/s/sess-a/repo/src/lib.rs");
        assert_eq!(drained[0].generation, 2);
    }

    #[test]
    fn interactive_qos_is_never_starved_by_bulk_transfers() {
        let gate = Arc::new(QosGate::new(1));

        // Hold the single Bulk permit to simulate an in-flight 1 GiB background stream.
        let bulk_permit = gate.acquire(TransportQoS::Bulk);

        // Spawn a second Bulk worker that must wait while bulk_permit is held.
        let gate_bulk2 = Arc::clone(&gate);
        let bulk2_acquired = Arc::new(AtomicBool::new(false));
        let bulk2_flag = Arc::clone(&bulk2_acquired);
        let bulk2_handle = thread::spawn(move || {
            let _p = gate_bulk2.acquire(TransportQoS::Bulk);
            bulk2_flag.store(true, Ordering::SeqCst);
        });

        // Give bulk2 a moment to block on the QoS gate.
        thread::sleep(Duration::from_millis(10));
        assert!(!bulk2_acquired.load(Ordering::SeqCst));

        // Interactive request acquires immediately without waiting on bulk!
        let t0 = Instant::now();
        {
            let _interactive = gate.acquire(TransportQoS::Interactive);
            assert!(
                t0.elapsed() < Duration::from_millis(10),
                "Interactive QoS was delayed by Bulk transfer!"
            );
        }

        // Release initial Bulk permit so bulk2 completes cleanly.
        drop(bulk_permit);
        bulk2_handle.join().unwrap();
        assert!(bulk2_acquired.load(Ordering::SeqCst));

        let stats = gate.stats();
        assert_eq!(stats.interactive_dispatched, 1);
        assert_eq!(stats.bulk_dispatched, 2);
        assert!(stats.bulk_yields >= 1);
    }

    // -------------------------------------------------------------------------
    // T9: Subtree WriteLease (sed -i & cp -r), EACCES + requests/<id>.json,
    //     ESTALE, turn-end revocation, and AWAY journal
    // -------------------------------------------------------------------------

    #[test]
    fn subtree_write_lease_sed_i_cp_r_eacces_ask_estale_turn_end_and_away_journal() {
        let tmp = tempfile::tempdir().unwrap();
        let req_dir = tmp.path().join("requests");
        let journal_file = tmp.path().join("away/journal.jsonl");

        let now_ms = Arc::new(AtomicU64::new(10_000));
        let clock_val = Arc::clone(&now_ms);
        let clock: Arc<dyn Fn() -> u64 + Send + Sync> =
            Arc::new(move || clock_val.load(Ordering::SeqCst));

        let host_reachable = Arc::new(AtomicBool::new(false));
        let leases = Arc::new(LeaseTable::new());
        let journal = Arc::new(AwayJournal::with_file(&journal_file));
        let hook = Arc::new(
            HumataAccessHook::new(Arc::clone(&leases), Arc::clone(&clock), Arc::clone(&host_reachable))
                .with_requests_dir(&req_dir)
                .with_away_journal(Arc::clone(&journal)),
        );
        hook.register_caller_session(700, "sess-b");

        let topo = sample_topology(Arc::new(AtomicUsize::new(0)));
        let vol = HumataTreeProjector::project(
            &topo,
            Some(Arc::clone(&leases)),
            Some(hook.clone()),
            Some(Arc::clone(&clock)),
        )
        .unwrap();

        let caller = CallerContext::new(700, 501, 20);

        // Grant ONE turn-bounded subtree lease on /s/sess-a/repo (clamped to 10m).
        let lease = hook.grant_turn_lease(
            700,
            VirtualPath::parse("/s/sess-a/repo").unwrap(),
            99 * 60 * 1000, // exceeds 10m -> clamped to 10m
            "turn-42",
        );
        assert_eq!(lease.expires_at_ms, 10_000 + MAX_TURN_LEASE_DURATION_MS);

        // 1. `sed -i` pattern: write temp file in same directory + atomic rename over target.
        let src_dir = vol.resolve_path(&caller, "/s/sess-a/repo/src").unwrap();
        let temp_attr = vol.resolve_path(&caller, "/s/sess-a/repo/src/temp.rs").unwrap();
        let fh = vol.open(&caller, temp_attr.ino, OpenFlags::read_write()).unwrap();
        vol.write(&caller, temp_attr.ino, fh, 0, b"pub fn updated() {}\n", Some(temp_attr.generation))
            .unwrap();
        vol.release(&caller, temp_attr.ino, fh).unwrap();
        vol.rename(&caller, src_dir.ino, "temp.rs", src_dir.ino, "lib.rs")
            .unwrap();
        let updated_lib = vol.resolve_path(&caller, "/s/sess-a/repo/src/lib.rs").unwrap();
        assert_eq!(
            read_all_bytes(&vol, &caller, &updated_lib).unwrap(),
            b"pub fn updated() {}\n"
        );

        // 2. `cp -r` pattern: mkdir nested subdirs + copy_file_range under the SAME lease.
        let repo_dir = vol.resolve_path(&caller, "/s/sess-a/repo").unwrap();
        let backup = vol.mkdir(&caller, repo_dir.ino, "backup", 0o755).unwrap();
        let _nested = vol.mkdir(&caller, backup.ino, "sub", 0o755).unwrap();

        // 3. Concurrent edit with stale `expected_generation` fails ESTALE!
        let stale_gen = updated_lib.generation.saturating_sub(1);
        let fh2 = vol.open(&caller, updated_lib.ino, OpenFlags::read_write()).unwrap();
        let estale_err = vol
            .write(&caller, updated_lib.ino, fh2, 0, b"conflict", Some(stale_gen))
            .unwrap_err();
        assert!(matches!(estale_err, VfsError::Estale { .. }));
        vol.release(&caller, updated_lib.ino, fh2).unwrap();

        // 4. Write outside leased subtree fails < 50ms with EACCES + writes requests/<id>.json.
        let other_repo = vol.resolve_path(&caller, "/s/sess-b/repo").unwrap();
        let t0 = Instant::now();
        let denied = vol.mkdir(&caller, other_repo.ino, "evil", 0o755).unwrap_err();
        let deny_elapsed = t0.elapsed();
        assert!(matches!(denied, VfsError::Eacces { .. }));
        assert!(
            deny_elapsed < Duration::from_millis(50),
            "EACCES denial took {deny_elapsed:?}"
        );

        let reqs = hook.list_requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].caller_pid, 700);
        assert_eq!(reqs[0].caller_session_id.as_deref(), Some("sess-b"));
        assert_eq!(reqs[0].target_session_id.as_deref(), Some("sess-b"));
        assert_eq!(reqs[0].target_path, "/s/sess-b/repo/evil");
        let on_disk = req_dir.join(reqs[0].filename());
        assert!(on_disk.exists(), "requests/<id>.json must be written to disk");

        // AWAY journal recorded both local writes and denied asks while host_reachable == false.
        assert!(!journal.is_empty());
        assert!(journal_file.exists());
        assert!(journal.entries().iter().any(|e| e.outcome == "denied_ask_created"));
        assert!(journal.entries().iter().any(|e| e.outcome == "allowed_local_write"));

        // 5. Turn-end revokes lease; subsequent writes inside /s/sess-a/repo fail EACCES.
        assert_eq!(hook.on_turn_end("turn-42"), 1);
        let after_revoke = vol.mkdir(&caller, repo_dir.ino, "after-turn", 0o755).unwrap_err();
        assert!(matches!(after_revoke, VfsError::Eacces { .. }));
    }

    // -------------------------------------------------------------------------
    // T10: MASSDELETE EPERM on all protected roots
    // -------------------------------------------------------------------------

    #[test]
    fn massdelete_enforces_eperm_on_all_protected_roots() {
        let leases = Arc::new(LeaseTable::new());
        let clock: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(|| 1_000);
        let host_reachable = Arc::new(AtomicBool::new(true));
        let hook = Arc::new(HumataAccessHook::new(
            Arc::clone(&leases),
            Arc::clone(&clock),
            host_reachable,
        ));

        // Even if caller holds a broad write lease on "/", MASSDELETE must still return EPERM!
        leases.grant(888, VirtualPath::root(), 1_000_000, Some("turn-broad".into()));

        let topo = sample_topology(Arc::new(AtomicUsize::new(0)));
        let vol = HumataTreeProjector::project(
            &topo,
            Some(leases),
            Some(hook),
            Some(clock),
        )
        .unwrap();
        let caller = CallerContext::new(888, 501, 20);

        // Protected directory roots (rmdir & rename must fail EPERM).
        let protected_dirs = [
            ("/", "s"),
            ("/", "m"),
            ("/", "channels"),
            ("/", "desks"),
            ("/", "machines"),
            ("/s", "sess-a"),
            ("/s/sess-a", "repo"),
            ("/s/sess-a", "config"),
            ("/s/sess-a/config", "claude"),
            ("/channels", "eng-main"),
            ("/desks", "desk-1"),
            ("/m", "mac-local"),
            ("/machines", "mac-local"),
        ];
        for (parent_path, child) in protected_dirs {
            let parent = vol.resolve_path(&caller, parent_path).unwrap();
            let rmdir_err = vol.rmdir(&caller, parent.ino, child).unwrap_err();
            assert!(
                matches!(rmdir_err, VfsError::Eperm { .. }),
                "expected EPERM for rmdir {parent_path}/{child}, got {rmdir_err:?}"
            );
            let rename_err = vol
                .rename(&caller, parent.ino, child, parent.ino, "moved-away")
                .unwrap_err();
            assert!(
                matches!(rename_err, VfsError::Eperm { .. }),
                "expected EPERM for rename {parent_path}/{child}, got {rename_err:?}"
            );
        }

        // Protected symlink alias roots (unlink & rename must fail EPERM).
        let protected_aliases = [
            ("/channels/eng-main", "sess-a"),
            ("/desks/desk-1", "sess-a"),
            ("/m/mac-local/s", "sess-a"),
        ];
        for (parent_path, child) in protected_aliases {
            let parent = vol.resolve_path(&caller, parent_path).unwrap();
            let unlink_err = vol.unlink(&caller, parent.ino, child).unwrap_err();
            assert!(
                matches!(unlink_err, VfsError::Eperm { .. }),
                "expected EPERM for unlink {parent_path}/{child}, got {unlink_err:?}"
            );
            let rename_err = vol
                .rename(&caller, parent.ino, child, parent.ino, "renamed-alias")
                .unwrap_err();
            assert!(
                matches!(rename_err, VfsError::Eperm { .. }),
                "expected EPERM for rename {parent_path}/{child}, got {rename_err:?}"
            );
        }
    }

    // -------------------------------------------------------------------------
    // T6: Daemon reconnect survival & same-machine channel rename (LIVEHOLDER)
    // -------------------------------------------------------------------------

    #[test]
    fn daemon_reconnect_and_channel_rename_preserve_live_holder_pid_and_paths() {
        let topo = sample_topology(Arc::new(AtomicUsize::new(0)));
        let host = HumataVolumeHost::new(1001, topo.clone(), None, None, None).unwrap();
        let caller = CallerContext::new(500, 501, 20);

        // Initial check: session is live with holder_pid 4242.
        assert_eq!(host.holder_record("sess-a").unwrap().holder_pid, 4242);
        assert!(host.daemon_link_state().is_connected());

        // Simulate humatad2 kill / disconnect: volume stays mounted and serving!
        let disconnected = host.on_daemon_disconnect();
        assert!(!disconnected.is_connected());
        let lib_during_disconnect = host
            .resolve_path(&caller, "/s/sess-a/repo/src/lib.rs")
            .unwrap();
        assert_eq!(
            read_all_bytes(&host, &caller, &lib_during_disconnect).unwrap(),
            b"pub fn hello() {}\n"
        );
        assert_eq!(host.holder_record("sess-a").unwrap().holder_pid, 4242);

        // Simulate restarted humatad2 reconnecting with a new daemon PID (2002).
        let mut refreshed = topo;
        // Even if the daemon sends a placeholder pid, host preserves live holder PID!
        refreshed.sessions[0].holder_pid = 9999;
        let reconnected = host.on_daemon_reconnect(2002, refreshed).unwrap();
        assert_eq!(
            reconnected,
            DaemonLinkState::Connected {
                daemon_pid: 2002,
                epoch: 2
            }
        );
        assert_eq!(host.reconnect_count(), 1);
        assert_eq!(host.holder_record("sess-a").unwrap().holder_pid, 4242);
        assert_eq!(
            host.holder_record("sess-a").unwrap().native_path,
            PathBuf::from("/Users/dev/humata/.sessions/sess-a")
        );

        // Same-machine channel rename: /channels/eng-main -> /channels/eng-platform.
        let updated = host
            .rename_channel_same_machine("eng-main", "eng-platform")
            .unwrap();
        assert_eq!(updated, 1);

        // New channel alias points to /s/sess-a; canonical path & holder_pid unchanged!
        let new_alias = host
            .resolve_path(&caller, "/channels/eng-platform/sess-a")
            .unwrap();
        assert_eq!(host.readlink(&caller, new_alias.ino).unwrap(), "/s/sess-a");
        assert!(host.resolve_path(&caller, "/channels/eng-main/sess-a").is_err());
        assert_eq!(host.holder_record("sess-a").unwrap().holder_pid, 4242);
        assert_eq!(
            host.holder_record("sess-a").unwrap().native_path,
            PathBuf::from("/Users/dev/humata/.sessions/sess-a")
        );
    }

    // -------------------------------------------------------------------------
    // T10: fs status & fs materialize byte-count confirmation planner
    // -------------------------------------------------------------------------

    #[test]
    fn materialize_planner_estimates_bytes_without_chunk_reads_and_requires_confirmation() {
        let lazy_hits = Arc::new(AtomicUsize::new(0));
        let topo = sample_topology(Arc::clone(&lazy_hits));
        let vol = HumataTreeProjector::project(&topo, None, None, None).unwrap();
        let caller = CallerContext::new(500, 501, 20);

        let planner = MaterializePlanner::default();
        let plan = planner
            .plan_from_volume(&vol, &caller, "/s/sess-a/repo")
            .unwrap();

        // Planning must NEVER trigger lazy file chunk downloads.
        assert_eq!(lazy_hits.load(Ordering::SeqCst), 0);
        assert_eq!(plan.targets.len(), 3);
        assert_eq!(plan.total_bytes, 2_147_483_648 + 524_288_000 + 128_000_000);
        assert_eq!(plan.total_entries, 18_400 + 42_000 + 9_500);
        assert!(plan.requires_confirmation);

        // Refuses execution without explicit confirmation.
        let err = planner
            .execute_plan(&plan, false, |t| Ok(t.estimated_bytes))
            .unwrap_err();
        assert!(matches!(err, MaterializeError::ConfirmationRequired { .. }));

        // Succeeds with explicit confirmation.
        let report = planner
            .execute_plan(&plan, true, |t| Ok(t.estimated_bytes))
            .unwrap();
        assert_eq!(report.materialized_targets, 3);
        assert_eq!(report.materialized_bytes, plan.total_bytes);

        // Status summary formats cleanly.
        let summary = FsStatusSummary {
            mode: FsMountMode::FsKit,
            mount_state: MountState::Healthy,
            mountpoint: PathBuf::from("/Users/dev/humata/mnt"),
            nobrowse: true,
            host_reachable: true,
            daemon_connected: true,
            active_sessions: 2,
            active_leases: 1,
            pending_requests: 0,
            cached_chunks: 128,
            cached_bytes: 32 * 1024 * 1024,
            cache_budget_bytes: 2 * 1024 * 1024 * 1024,
            queued_invalidations: 0,
            away_journal_entries: 0,
        };
        let text = summary.format_human();
        assert!(text.contains("mode:                 fskit"));
        assert!(text.contains("32.00 MiB"));
    }
}
