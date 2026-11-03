//! The Rust part of the polyglot example; see polyglot.h.

use std::ffi::{CStr, c_char};

const NAME: &CStr = c"Rust";

/// Returns "Rust" as a static C string.
#[unsafe(no_mangle)]
pub extern "C" fn rust_name() -> *const c_char {
    NAME.as_ptr()
}
