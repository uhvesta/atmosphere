//! A SQLite VFS over the default one that tells the ledger what each write
//! to a database or WAL file replaces, and when the file is synced.
//!
//! Everything else goes to the real VFS as it is. The shared-memory index
//! isn't tracked: it is rebuilt from the WAL by the first connection after
//! a crash, as it is after a real power cut.

use std::ffi::{CStr, OsStr, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;
use std::sync::Once;

use libsqlite3_sys as ffi;

use crate::ledger::{self, Id};

/// Makes the shim SQLite's default VFS, so every connection opened after
/// uses it. Does nothing without a power cut being simulated. Call before
/// opening a connection; calling it again changes nothing.
pub fn install() {
    static INSTALLED: Once = Once::new();
    if std::env::var_os("HUMATA2_POWER_CUT_ROOTS").is_none() {
        return;
    }
    INSTALLED.call_once(|| {
        // SAFETY: sqlite3_vfs_find initializes SQLite if needed and
        // returns the default VFS, which lives as long as the process.
        // The shim is leaked, so it does too.
        unsafe {
            let real = ffi::sqlite3_vfs_find(ptr::null());
            assert!(!real.is_null(), "SQLite has no default VFS");
            let mut shim: ffi::sqlite3_vfs = ptr::read(real);
            shim.szOsFile = c_int::try_from(SHIM_SIZE).unwrap_or(c_int::MAX) + (*real).szOsFile;
            shim.pNext = ptr::null_mut();
            shim.zName = c"humata2-power-cut".as_ptr();
            shim.pAppData = real.cast();
            shim.xOpen = Some(x_open);
            let shim = Box::into_raw(Box::new(shim));
            let rc = ffi::sqlite3_vfs_register(shim, 1);
            assert_eq!(rc, ffi::SQLITE_OK, "couldn't register the power-cut VFS");
        }
    });
}

/// What the shim keeps for a file, before the real VFS's own.
#[repr(C)]
struct Shim {
    base: ffi::sqlite3_file,
    /// The file, if writes to it are tracked.
    tracked: Option<Id>,
}

/// Rounded up so the real file after it is aligned.
const SHIM_SIZE: usize = std::mem::size_of::<Shim>().next_multiple_of(16);

/// The real VFS's file, after the shim's.
unsafe fn real(file: *mut ffi::sqlite3_file) -> *mut ffi::sqlite3_file {
    // SAFETY: SQLite gave `file` szOsFile bytes, SHIM_SIZE of them ours.
    unsafe { file.cast::<u8>().add(SHIM_SIZE).cast() }
}

unsafe fn methods(file: *mut ffi::sqlite3_file) -> &'static ffi::sqlite3_io_methods {
    // SAFETY: an open real file always has its methods.
    unsafe { &*(*real(file)).pMethods }
}

unsafe fn tracked(file: *mut ffi::sqlite3_file) -> Option<Id> {
    // SAFETY: set by x_open before SQLite can call any method.
    unsafe { (*file.cast::<Shim>()).tracked }
}

unsafe extern "C" fn x_open(
    vfs: *mut ffi::sqlite3_vfs,
    name: ffi::sqlite3_filename,
    file: *mut ffi::sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    // SAFETY: pAppData is the real VFS, set in install; SQLite gave
    // `file` szOsFile bytes.
    unsafe {
        let real_vfs: *mut ffi::sqlite3_vfs = (*vfs).pAppData.cast();
        (*file).pMethods = ptr::null();
        let Some(open) = (*real_vfs).xOpen else {
            return ffi::SQLITE_ERROR;
        };
        let rc = open(real_vfs, name, real(file), flags, out_flags);
        if rc != ffi::SQLITE_OK || (*real(file)).pMethods.is_null() {
            return rc;
        }
        let kinds = ffi::SQLITE_OPEN_MAIN_DB | ffi::SQLITE_OPEN_WAL | ffi::SQLITE_OPEN_MAIN_JOURNAL;
        let tracked = if flags & kinds != 0 && !name.is_null() {
            let path = PathBuf::from(OsStr::from_bytes(CStr::from_ptr(name).to_bytes()));
            ledger::id_of_path(&path)
                .ok()
                .filter(|id| ledger::track(&path, *id))
        } else {
            None
        };
        ptr::write(ptr::addr_of_mut!((*file.cast::<Shim>()).tracked), tracked);
        (*file).pMethods = &METHODS;
        ffi::SQLITE_OK
    }
}

static METHODS: ffi::sqlite3_io_methods = ffi::sqlite3_io_methods {
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

/// Calls the real file's method `$name`, or returns `$missing` when it has
/// none.
macro_rules! forward {
    ($file:expr, $name:ident, $missing:expr $(, $arg:expr)*) => {
        // SAFETY: SQLite only calls methods on a file x_open opened.
        unsafe {
            match methods($file).$name {
                Some(method) => method(real($file) $(, $arg)*),
                None => $missing,
            }
        }
    };
}

unsafe extern "C" fn x_close(file: *mut ffi::sqlite3_file) -> c_int {
    forward!(file, xClose, ffi::SQLITE_OK)
}

unsafe extern "C" fn x_read(file: *mut ffi::sqlite3_file, buf: *mut c_void, amount: c_int, offset: i64) -> c_int {
    forward!(file, xRead, ffi::SQLITE_IOERR_READ, buf, amount, offset)
}

fn io_error(rc: c_int) -> std::io::Error {
    std::io::Error::other(format!("SQLite I/O error {rc}"))
}

unsafe fn size_of_file(file: *mut ffi::sqlite3_file) -> std::io::Result<u64> {
    let mut size: i64 = 0;
    let rc = forward!(file, xFileSize, ffi::SQLITE_IOERR_FSTAT, &mut size);
    if rc == ffi::SQLITE_OK {
        Ok(u64::try_from(size).unwrap_or(0))
    } else {
        Err(io_error(rc))
    }
}

unsafe fn read_old(file: *mut ffi::sqlite3_file, at: u64, len: usize) -> std::io::Result<Vec<u8>> {
    let mut old = vec![0u8; len];
    let amount = c_int::try_from(len).map_err(|_| io_error(ffi::SQLITE_IOERR_READ))?;
    let offset = i64::try_from(at).map_err(|_| io_error(ffi::SQLITE_IOERR_READ))?;
    let rc = forward!(file, xRead, ffi::SQLITE_IOERR_READ, old.as_mut_ptr().cast(), amount, offset);
    if rc == ffi::SQLITE_OK {
        Ok(old)
    } else {
        Err(io_error(rc))
    }
}

/// Runs `change` with what it replaces remembered, if the file is tracked.
unsafe fn changing(file: *mut ffi::sqlite3_file, offset: i64, replaced: u64, change: impl FnOnce() -> c_int) -> c_int {
    // SAFETY: as for every method.
    let Some(id) = (unsafe { tracked(file) }) else {
        return change();
    };
    let offset = u64::try_from(offset).unwrap_or(0);
    let done = ledger::changing(
        id,
        offset,
        replaced,
        // SAFETY: as for every method.
        || unsafe { size_of_file(file) },
        |at, len| unsafe { read_old(file, at, len) },
        || Ok(change()),
    );
    done.unwrap_or(ffi::SQLITE_IOERR)
}

unsafe extern "C" fn x_write(file: *mut ffi::sqlite3_file, buf: *const c_void, amount: c_int, offset: i64) -> c_int {
    let replaced = u64::try_from(amount).unwrap_or(0);
    let write = || forward!(file, xWrite, ffi::SQLITE_IOERR_WRITE, buf, amount, offset);
    // SAFETY: as for every method.
    unsafe { changing(file, offset, replaced, write) }
}

unsafe extern "C" fn x_truncate(file: *mut ffi::sqlite3_file, size: i64) -> c_int {
    let truncate = || forward!(file, xTruncate, ffi::SQLITE_IOERR_TRUNCATE, size);
    // SAFETY: as for every method.
    unsafe { changing(file, size, u64::MAX, truncate) }
}

unsafe extern "C" fn x_sync(file: *mut ffi::sqlite3_file, flags: c_int) -> c_int {
    let rc = forward!(file, xSync, ffi::SQLITE_IOERR_FSYNC, flags);
    // SAFETY: as for every method.
    if rc == ffi::SQLITE_OK
        && let Some(id) = unsafe { tracked(file) }
    {
        ledger::synced(id);
    }
    rc
}

unsafe extern "C" fn x_file_size(file: *mut ffi::sqlite3_file, size: *mut i64) -> c_int {
    forward!(file, xFileSize, ffi::SQLITE_IOERR_FSTAT, size)
}

unsafe extern "C" fn x_lock(file: *mut ffi::sqlite3_file, level: c_int) -> c_int {
    forward!(file, xLock, ffi::SQLITE_IOERR_LOCK, level)
}

unsafe extern "C" fn x_unlock(file: *mut ffi::sqlite3_file, level: c_int) -> c_int {
    forward!(file, xUnlock, ffi::SQLITE_IOERR_UNLOCK, level)
}

unsafe extern "C" fn x_check_reserved_lock(file: *mut ffi::sqlite3_file, out: *mut c_int) -> c_int {
    forward!(file, xCheckReservedLock, ffi::SQLITE_IOERR_CHECKRESERVEDLOCK, out)
}

unsafe extern "C" fn x_file_control(file: *mut ffi::sqlite3_file, op: c_int, arg: *mut c_void) -> c_int {
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
    forward!(file, xShmMap, ffi::SQLITE_IOERR_SHMMAP, region, size, extend, out)
}

unsafe extern "C" fn x_shm_lock(file: *mut ffi::sqlite3_file, offset: c_int, n: c_int, flags: c_int) -> c_int {
    forward!(file, xShmLock, ffi::SQLITE_IOERR_SHMLOCK, offset, n, flags)
}

unsafe extern "C" fn x_shm_barrier(file: *mut ffi::sqlite3_file) {
    forward!(file, xShmBarrier, ())
}

unsafe extern "C" fn x_shm_unmap(file: *mut ffi::sqlite3_file, delete: c_int) -> c_int {
    forward!(file, xShmUnmap, ffi::SQLITE_OK, delete)
}

unsafe extern "C" fn x_fetch(file: *mut ffi::sqlite3_file, offset: i64, amount: c_int, out: *mut *mut c_void) -> c_int {
    // Memory-mapped reads are off for tracked files: a write through the
    // map would bypass x_write.
    // SAFETY: `out` is SQLite's to fill.
    unsafe {
        if tracked(file).is_some() {
            *out = ptr::null_mut();
            return ffi::SQLITE_OK;
        }
    }
    forward!(file, xFetch, ffi::SQLITE_OK, offset, amount, out)
}

unsafe extern "C" fn x_unfetch(file: *mut ffi::sqlite3_file, offset: i64, page: *mut c_void) -> c_int {
    forward!(file, xUnfetch, ffi::SQLITE_OK, offset, page)
}
