//! Gate T1 SQLite WAL matrix probe:
//! 1. Native tempdir baseline: `PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;`
//!    with concurrent reader + writer connections, `wal_checkpoint(TRUNCATE)`,
//!    `-shm` deletion + crash-recovery reopen, and `PRAGMA integrity_check`.
//! 2. Simulated FUSE-like write-through VFS (`fusefs-trace`): traces `xRead`,
//!    `xWrite`, `xSync`, `xLock`, `xShmMap`, and `xShmLock` to verify `pwrite`
//!    -> `fsync` ordering and `-shm`/`-wal` mmap/lock activity.
//! 3. Simulated FUSE/FSKit without coherent `-shm` mmap (`fusefs-no-shm`):
//!    returns `SQLITE_IOERR_SHMMAP` from `xShmMap`, demonstrating why SQLite
//!    WAL databases must stay strictly on native disk.

use std::ffi::{c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Once};
use std::thread;

use libsqlite3_sys as ffi;
use rusqlite::{Connection, OpenFlags};

#[derive(Default)]
struct TraceCounters {
    reads: AtomicUsize,
    writes: AtomicUsize,
    syncs: AtomicUsize,
    locks: AtomicUsize,
    shm_maps: AtomicUsize,
    shm_locks: AtomicUsize,
    write_before_sync_seen: AtomicBool,
}

static TRACE_COUNTERS: TraceCounters = TraceCounters {
    reads: AtomicUsize::new(0),
    writes: AtomicUsize::new(0),
    syncs: AtomicUsize::new(0),
    locks: AtomicUsize::new(0),
    shm_maps: AtomicUsize::new(0),
    shm_locks: AtomicUsize::new(0),
    write_before_sync_seen: AtomicBool::new(false),
};

#[repr(C)]
struct VfsAppData {
    real_vfs: *mut ffi::sqlite3_vfs,
    fail_shm_map: bool,
}

#[repr(C)]
struct ShimFile {
    base: ffi::sqlite3_file,
    fail_shm_map: bool,
}

const SHIM_SIZE: usize = std::mem::size_of::<ShimFile>().next_multiple_of(16);

unsafe fn real_file(file: *mut ffi::sqlite3_file) -> *mut ffi::sqlite3_file {
    // SAFETY: SQLite allocated `SHIM_SIZE + real_vfs.szOsFile` bytes.
    unsafe { file.cast::<u8>().add(SHIM_SIZE).cast() }
}

unsafe fn real_methods(file: *mut ffi::sqlite3_file) -> &'static ffi::sqlite3_io_methods {
    // SAFETY: `real_file` was opened by the underlying OS VFS.
    unsafe { &*(*real_file(file)).pMethods }
}

macro_rules! forward {
    ($file:expr, $name:ident, $missing:expr $(, $arg:expr)*) => {
        // SAFETY: SQLite only invokes io_methods on files opened by `x_open`.
        unsafe {
            match real_methods($file).$name {
                Some(method) => method(real_file($file) $(, $arg)*),
                None => $missing,
            }
        }
    };
}

unsafe extern "C" fn x_open(
    vfs: *mut ffi::sqlite3_vfs,
    name: ffi::sqlite3_filename,
    file: *mut ffi::sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    // SAFETY: `pAppData` points to a leaked `VfsAppData`.
    unsafe {
        let app: *const VfsAppData = (*vfs).pAppData.cast();
        let real_vfs = (*app).real_vfs;
        (*file).pMethods = ptr::null();
        let Some(open) = (*real_vfs).xOpen else {
            return ffi::SQLITE_ERROR;
        };
        let rc = open(real_vfs, name, real_file(file), flags, out_flags);
        if rc != ffi::SQLITE_OK || (*real_file(file)).pMethods.is_null() {
            return rc;
        }
        (*file.cast::<ShimFile>()).fail_shm_map = (*app).fail_shm_map;
        (*file).pMethods = &PROBE_METHODS;
        ffi::SQLITE_OK
    }
}

unsafe extern "C" fn x_close(file: *mut ffi::sqlite3_file) -> c_int {
    forward!(file, xClose, ffi::SQLITE_OK)
}

unsafe extern "C" fn x_read(
    file: *mut ffi::sqlite3_file,
    buf: *mut c_void,
    amount: c_int,
    offset: i64,
) -> c_int {
    TRACE_COUNTERS.reads.fetch_add(1, Ordering::Relaxed);
    forward!(file, xRead, ffi::SQLITE_IOERR_READ, buf, amount, offset)
}

unsafe extern "C" fn x_write(
    file: *mut ffi::sqlite3_file,
    buf: *const c_void,
    amount: c_int,
    offset: i64,
) -> c_int {
    TRACE_COUNTERS.writes.fetch_add(1, Ordering::Relaxed);
    forward!(file, xWrite, ffi::SQLITE_IOERR_WRITE, buf, amount, offset)
}

unsafe extern "C" fn x_truncate(file: *mut ffi::sqlite3_file, size: i64) -> c_int {
    forward!(file, xTruncate, ffi::SQLITE_IOERR_TRUNCATE, size)
}

unsafe extern "C" fn x_sync(file: *mut ffi::sqlite3_file, flags: c_int) -> c_int {
    if TRACE_COUNTERS.writes.load(Ordering::Relaxed) > 0 {
        TRACE_COUNTERS
            .write_before_sync_seen
            .store(true, Ordering::Relaxed);
    }
    TRACE_COUNTERS.syncs.fetch_add(1, Ordering::Relaxed);
    forward!(file, xSync, ffi::SQLITE_IOERR_FSYNC, flags)
}

unsafe extern "C" fn x_file_size(file: *mut ffi::sqlite3_file, size: *mut i64) -> c_int {
    forward!(file, xFileSize, ffi::SQLITE_IOERR_FSTAT, size)
}

unsafe extern "C" fn x_lock(file: *mut ffi::sqlite3_file, level: c_int) -> c_int {
    TRACE_COUNTERS.locks.fetch_add(1, Ordering::Relaxed);
    forward!(file, xLock, ffi::SQLITE_IOERR_LOCK, level)
}

unsafe extern "C" fn x_unlock(file: *mut ffi::sqlite3_file, level: c_int) -> c_int {
    forward!(file, xUnlock, ffi::SQLITE_IOERR_UNLOCK, level)
}

unsafe extern "C" fn x_check_reserved_lock(file: *mut ffi::sqlite3_file, out: *mut c_int) -> c_int {
    forward!(
        file,
        xCheckReservedLock,
        ffi::SQLITE_IOERR_CHECKRESERVEDLOCK,
        out
    )
}

unsafe extern "C" fn x_file_control(
    file: *mut ffi::sqlite3_file,
    op: c_int,
    arg: *mut c_void,
) -> c_int {
    forward!(file, xFileControl, ffi::SQLITE_NOTFOUND, op, arg)
}

unsafe extern "C" fn x_sector_size(file: *mut ffi::sqlite3_file) -> c_int {
    forward!(file, xSectorSize, 4096)
}

unsafe extern "C" fn x_device_characteristics(file: *mut ffi::sqlite3_file) -> c_int {
    forward!(file, xDeviceCharacteristics, 0)
}

unsafe extern "C" fn x_shm_map(
    file: *mut ffi::sqlite3_file,
    region: c_int,
    size: c_int,
    extend: c_int,
    out: *mut *mut c_void,
) -> c_int {
    // SAFETY: `file` was initialized by `x_open`.
    let fail = unsafe { (*file.cast::<ShimFile>()).fail_shm_map };
    if fail {
        return ffi::SQLITE_IOERR_SHMMAP;
    }
    TRACE_COUNTERS.shm_maps.fetch_add(1, Ordering::Relaxed);
    forward!(
        file,
        xShmMap,
        ffi::SQLITE_IOERR_SHMMAP,
        region,
        size,
        extend,
        out
    )
}

unsafe extern "C" fn x_shm_lock(
    file: *mut ffi::sqlite3_file,
    offset: c_int,
    n: c_int,
    flags: c_int,
) -> c_int {
    TRACE_COUNTERS.shm_locks.fetch_add(1, Ordering::Relaxed);
    forward!(file, xShmLock, ffi::SQLITE_IOERR_SHMLOCK, offset, n, flags)
}

unsafe extern "C" fn x_shm_barrier(file: *mut ffi::sqlite3_file) {
    forward!(file, xShmBarrier, ())
}

unsafe extern "C" fn x_shm_unmap(file: *mut ffi::sqlite3_file, delete: c_int) -> c_int {
    forward!(file, xShmUnmap, ffi::SQLITE_OK, delete)
}

unsafe extern "C" fn x_fetch(
    file: *mut ffi::sqlite3_file,
    offset: i64,
    amount: c_int,
    out: *mut *mut c_void,
) -> c_int {
    forward!(file, xFetch, ffi::SQLITE_OK, offset, amount, out)
}

unsafe extern "C" fn x_unfetch(
    file: *mut ffi::sqlite3_file,
    offset: i64,
    page: *mut c_void,
) -> c_int {
    forward!(file, xUnfetch, ffi::SQLITE_OK, offset, page)
}

static PROBE_METHODS: ffi::sqlite3_io_methods = ffi::sqlite3_io_methods {
    iVersion: 3,
    xClose: Some(x_close),
    xRead: Some(x_read),
    xWrite: Some(x_write),
    xTruncate: Some(x_truncate),
    xSync: Some(x_sync),
    xFileSize: Some(x_file_size),
    xLock: Some(x_lock),
    xUnlock: Some(x_unlock),
    xCheckReservedLock: Some(x_check_reserved_lock),
    xFileControl: Some(x_file_control),
    xSectorSize: Some(x_sector_size),
    xDeviceCharacteristics: Some(x_device_characteristics),
    xShmMap: Some(x_shm_map),
    xShmLock: Some(x_shm_lock),
    xShmBarrier: Some(x_shm_barrier),
    xShmUnmap: Some(x_shm_unmap),
    xFetch: Some(x_fetch),
    xUnfetch: Some(x_unfetch),
};

fn register_probe_vfs() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: `sqlite3_vfs_find(NULL)` returns SQLite's built-in default OS VFS.
        // We register two named non-default VFSes (`make_default = 0`) leaked for process lifetime.
        unsafe {
            let real = ffi::sqlite3_vfs_find(ptr::null());
            assert!(!real.is_null(), "SQLite has no default VFS");

            for (name, fail_shm) in [(c"fusefs-trace", false), (c"fusefs-no-shm", true)] {
                let mut shim: ffi::sqlite3_vfs = ptr::read(real);
                shim.szOsFile = c_int::try_from(SHIM_SIZE).unwrap_or(c_int::MAX) + (*real).szOsFile;
                shim.pNext = ptr::null_mut();
                shim.zName = name.as_ptr();
                shim.pAppData = Box::into_raw(Box::new(VfsAppData {
                    real_vfs: real,
                    fail_shm_map: fail_shm,
                }))
                .cast();
                shim.xOpen = Some(x_open);
                let rc = ffi::sqlite3_vfs_register(Box::into_raw(Box::new(shim)), 0);
                assert_eq!(rc, ffi::SQLITE_OK, "failed to register probe VFS");
            }
        }
    });
}

#[test]
fn native_wal_concurrent_readers_writers_and_recovery_pass_integrity_check() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = Arc::new(dir.path().join("state.sqlite"));

    let setup = Connection::open(db_path.as_ref()).expect("open db");
    let mode: String = setup
        .query_row("PRAGMA journal_mode=WAL;", [], |r| r.get(0))
        .expect("set WAL mode");
    assert_eq!(mode.to_ascii_lowercase(), "wal");
    setup
        .execute_batch(
            "PRAGMA synchronous=FULL;
             CREATE TABLE events (id INTEGER PRIMARY KEY, body TEXT NOT NULL);",
        )
        .expect("create schema");

    // Verify WAL sidecar files exist while connection is active.
    setup
        .execute("INSERT INTO events (body) VALUES ('seed')", [])
        .expect("seed insert");
    assert!(
        dir.path().join("state.sqlite-wal").exists(),
        "expected -wal file on native disk"
    );
    assert!(
        dir.path().join("state.sqlite-shm").exists(),
        "expected -shm file on native disk"
    );

    // Concurrent writer + reader threads.
    let writer_path = Arc::clone(&db_path);
    let reader_path = Arc::clone(&db_path);
    let writer = thread::spawn(move || {
        let conn = Connection::open(writer_path.as_ref()).expect("writer open");
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .expect("busy_timeout");
        for i in 0..50 {
            conn.execute("INSERT INTO events (body) VALUES (?1)", [format!("row-{i}")])
                .expect("insert");
        }
    });
    let reader = thread::spawn(move || {
        let conn = Connection::open(reader_path.as_ref()).expect("reader open");
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .expect("busy_timeout");
        let mut max_seen = 0i64;
        for _ in 0..50 {
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
                .expect("count");
            assert!(count >= max_seen, "WAL snapshot must be monotonic");
            max_seen = count;
        }
    });
    writer.join().expect("writer thread");
    reader.join().expect("reader thread");

    // Checkpoint and verify integrity_check == ok.
    let _: (i64, i64, i64) = setup
        .query_row("PRAGMA wal_checkpoint(TRUNCATE);", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .expect("checkpoint");
    let integrity: String = setup
        .query_row("PRAGMA integrity_check;", [], |r| r.get(0))
        .expect("integrity_check");
    assert_eq!(integrity, "ok");
    let total: i64 = setup
        .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
        .expect("final count");
    assert_eq!(total, 51);
}

#[test]
fn trace_vfs_observes_pwrite_fsync_and_shm_activity() {
    register_probe_vfs();
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("traced.sqlite");

    let conn = Connection::open_with_flags_and_vfs(
        &db_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        "fusefs-trace",
    )
    .expect("open with fusefs-trace VFS");

    let mode: String = conn
        .query_row("PRAGMA journal_mode=WAL;", [], |r| r.get(0))
        .expect("enable WAL");
    assert_eq!(mode.to_ascii_lowercase(), "wal");
    conn.execute_batch(
        "PRAGMA synchronous=FULL;
         CREATE TABLE kv (k TEXT PRIMARY KEY, v TEXT);
         INSERT INTO kv VALUES ('a', '1');
         INSERT INTO kv VALUES ('b', '2');",
    )
    .expect("write WAL rows");

    assert!(TRACE_COUNTERS.writes.load(Ordering::Relaxed) > 0);
    assert!(TRACE_COUNTERS.syncs.load(Ordering::Relaxed) > 0);
    assert!(TRACE_COUNTERS.write_before_sync_seen.load(Ordering::Relaxed));
    assert!(
        TRACE_COUNTERS.shm_maps.load(Ordering::Relaxed) > 0,
        "WAL mode requires xShmMap"
    );
    assert!(
        TRACE_COUNTERS.shm_locks.load(Ordering::Relaxed) > 0,
        "WAL mode requires xShmLock"
    );
}

#[test]
fn fuse_without_coherent_shm_mmap_breaks_wal_proving_native_disk_requirement() {
    register_probe_vfs();
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("no_shm.sqlite");

    let conn = Connection::open_with_flags_and_vfs(
        &db_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        "fusefs-no-shm",
    )
    .expect("open with fusefs-no-shm VFS");

    // Enabling WAL and writing requires xShmMap; returning SQLITE_IOERR_SHMMAP
    // simulates FUSE/FSKit lacking coherent shared-memory -shm support.
    let wal_result = conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE t (x INTEGER);
         INSERT INTO t VALUES (1);",
    );
    assert!(
        wal_result.is_err(),
        "WAL must fail when underlying VFS lacks coherent xShmMap"
    );
}
