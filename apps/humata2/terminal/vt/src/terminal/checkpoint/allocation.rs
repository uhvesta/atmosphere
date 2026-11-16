//! A native allocator whose accounting survives the decoded terminal.
use crate::{Error, Status, ffi};
use std::{
    alloc::{self, Layout},
    cell::Cell,
    ffi::c_void,
    ptr,
};

pub(super) const MEMORY_LIMIT: usize = 128 * 1024 * 1024;

struct State {
    limit: usize,
    used: Cell<usize>,
    rejected: Cell<Option<usize>>,
}

pub(in crate::terminal) struct AllocationBudget {
    pub(super) abi: ffi::GhosttyAllocator,
    state: State,
}

impl AllocationBudget {
    pub(super) fn new() -> Box<Self> {
        Self::with_limit(MEMORY_LIMIT)
    }

    fn with_limit(limit: usize) -> Box<Self> {
        let mut budget = Box::new(Self {
            abi: ffi::GhosttyAllocator {
                ctx: ptr::null_mut(),
                vtable: &VTABLE,
            },
            state: State {
                limit,
                used: Cell::new(0),
                rejected: Cell::new(None),
            },
        });
        // The allocation stays at this address even when its Box moves.
        budget.abi.ctx = (&raw mut budget.state).cast();
        budget
    }

    pub(super) fn error(&self, operation: &'static str, source: Status) -> Error {
        match self.state.rejected.get() {
            Some(requested) => Error::CheckpointMemoryLimit {
                requested,
                limit: self.state.limit,
                source,
            },
            None => Error::Checkpoint { operation, source },
        }
    }

    pub(super) fn rejected(&self) -> bool {
        self.state.rejected.get().is_some()
    }
}

static VTABLE: ffi::GhosttyAllocatorVtable = ffi::GhosttyAllocatorVtable {
    alloc: allocate,
    resize: None,
    remap: None,
    free: release,
    reserved: [ptr::null(); 4],
};

fn layout(len: usize, alignment: u8) -> Option<Layout> {
    let alignment = 1usize.checked_shl(u32::from(alignment))?;
    Layout::from_size_align(len.max(1), alignment).ok()
}

unsafe extern "C" fn allocate(
    ctx: *mut c_void,
    len: usize,
    alignment: u8,
    _: usize,
) -> *mut c_void {
    // SAFETY: ctx points into the stable live AllocationBudget; the native
    // terminal is exclusively owned during every allocation callback.
    let state = unsafe { &*ctx.cast::<State>() };
    let requested = len.max(1);
    let Some(total) = state
        .used
        .get()
        .checked_add(requested)
        .filter(|n| *n <= state.limit)
    else {
        state.rejected.set(Some(len));
        return ptr::null_mut();
    };
    let Some(layout) = layout(len, alignment) else {
        state.rejected.set(Some(len));
        return ptr::null_mut();
    };
    // SAFETY: the checked nonzero layout is valid. Returning NULL reports
    // allocation failure to Ghostty instead of invoking Rust's OOM handler.
    let memory = unsafe { alloc::alloc(layout) };
    if !memory.is_null() {
        state.used.set(total);
    }
    memory.cast()
}

unsafe extern "C" fn release(
    ctx: *mut c_void,
    memory: *mut c_void,
    len: usize,
    alignment: u8,
    _: usize,
) {
    if memory.is_null() {
        return;
    }
    // SAFETY: Ghostty passes the original length/alignment for a block from
    // this same allocator. The State outlives all its native allocations.
    let state = unsafe { &*ctx.cast::<State>() };
    if let Some(layout) = layout(len, alignment) {
        unsafe { alloc::dealloc(memory.cast(), layout) };
        state.used.set(state.used.get().saturating_sub(len.max(1)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_rejects_excess_and_released_memory_can_be_reused() {
        let budget = AllocationBudget::with_limit(8);
        // SAFETY: only tiny 8-byte allocations reach the system allocator;
        // this live budget owns every pointer and each is freed once.
        unsafe {
            let first = allocate(budget.abi.ctx, 8, 3, 0);
            assert!(!first.is_null());
            assert!((first as usize).is_multiple_of(8));
            assert!(allocate(budget.abi.ctx, usize::MAX, 3, 0).is_null());
            assert!(allocate(budget.abi.ctx, 1, 0, 0).is_null());
            release(budget.abi.ctx, first, 8, 3, 0);
            let second = allocate(budget.abi.ctx, 8, 3, 0);
            assert!(!second.is_null());
            release(budget.abi.ctx, second, 8, 3, 0);
        }
    }
}
