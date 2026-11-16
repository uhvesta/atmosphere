//! Complete engine snapshots. This is not a screen-text/ANSI approximation.
use super::{Owned, Terminal};
use crate::{Error, error::check, ffi};
use std::{ffi::c_void, ptr};

mod allocation;
mod preflight;
pub(super) use allocation::AllocationBudget;

pub(super) const CONTINUATION_LIMIT: usize = 1024 * 1024;
const INPUT_LIMIT: usize = 16 * 1024 * 1024;
const GRID_CELLS_LIMIT: usize = 1024 * 1024;
const GRID_DIMENSION_LIMIT: u16 = 4096;
const MAGIC: &[u8; 8] = b"GVTC\0\0\0\x01";
const HEADER: usize = 20;

pub(super) fn validate_grid(cols: u16, rows: u16) -> Result<(), Error> {
    if cols == 0
        || rows == 0
        || cols > GRID_DIMENSION_LIMIT
        || rows > GRID_DIMENSION_LIMIT
        || usize::from(cols) * usize::from(rows) > GRID_CELLS_LIMIT
    {
        return Err(Error::GridLimit {
            cols,
            rows,
            max_dimension: GRID_DIMENSION_LIMIT,
            max_cells: GRID_CELLS_LIMIT,
        });
    }
    Ok(())
}

impl Terminal {
    /// Serialize both screens, history, cursor, modes, colors, links and the
    /// unfinished VT/UTF-8 parser state. `max_bytes` includes the envelope.
    ///
    /// These are opaque versioned snapshot bytes, **not** bytes to `feed`.
    /// Store/transport the engine format identifier alongside them. The size
    /// limit is checked before allocating the result, and failure leaves this
    /// terminal and any older saved checkpoint unchanged.
    pub fn checkpoint(&self, max_bytes: usize) -> Result<Vec<u8>, Error> {
        let fail = |source| Error::Checkpoint {
            operation: "encode",
            source,
        };
        let mut needed = 0;
        // SAFETY: live terminal, NULL requests a size, needed is a valid output.
        let result = unsafe {
            ffi::ghostty_snapshot_encode_buf(self.raw.ptr, ptr::null_mut(), 0, &mut needed)
        };
        if result != ffi::OUT_OF_SPACE {
            check(result).map_err(fail)?;
        }
        let total = needed
            .checked_add(HEADER)
            .ok_or(Error::CheckpointTooLarge {
                required: usize::MAX,
                limit: max_bytes,
            })?;
        if total > max_bytes {
            return Err(Error::CheckpointTooLarge {
                required: total,
                limit: max_bytes,
            });
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(total)
            .map_err(|source| Error::CheckpointAllocation {
                bytes: total,
                source,
            })?;
        bytes.resize(total, 0);
        bytes[..8].copy_from_slice(MAGIC);
        let size = self.size();
        bytes[8..10].copy_from_slice(&size.cols.to_le_bytes());
        bytes[10..12].copy_from_slice(&size.rows.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.cell_width.to_le_bytes());
        bytes[16..20].copy_from_slice(&self.cell_height.to_le_bytes());
        let mut written = 0;
        // SAFETY: the output slice has the measured capacity; no terminal
        // mutation or callbacks can interleave on this non-Sync terminal.
        check(unsafe {
            ffi::ghostty_snapshot_encode_buf(
                self.raw.ptr,
                bytes[HEADER..].as_mut_ptr(),
                needed,
                &mut written,
            )
        })
        .map_err(fail)?;
        bytes.truncate(HEADER + written);
        Ok(bytes)
    }

    /// Restore an opaque checkpoint, with fresh local callbacks/encoders and
    /// no historical title, clipboard, bell or PTY reply side effects.
    /// The binary envelope uses fixed-width little-endian values on every OS.
    pub fn from_checkpoint(bytes: &[u8]) -> Result<Self, Error> {
        let bad = |reason| Error::CheckpointHeader { reason };
        if bytes.len() > INPUT_LIMIT {
            return Err(Error::CheckpointTooLarge {
                required: bytes.len(),
                limit: INPUT_LIMIT,
            });
        }
        if bytes.len() < HEADER || &bytes[..8] != MAGIC {
            return Err(bad("unknown envelope version or truncated header"));
        }
        let cols = u16::from_le_bytes([bytes[8], bytes[9]]);
        let rows = u16::from_le_bytes([bytes[10], bytes[11]]);
        let width = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let height = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        if cols == 0 || rows == 0 || width == 0 || height == 0 {
            return Err(bad("zero terminal dimensions"));
        }
        validate_grid(cols, rows)?;
        let body = &bytes[HEADER..];
        preflight::validate(body, cols, rows)?;
        // Native codecs can allocate from embedded lengths before their CRC
        // check; even a tiny corrupted private file needs a memory quota.
        let allocator = AllocationBudget::new();
        let fail = |source| allocator.error("decode", source);
        let decoder = Owned::new(
            // SAFETY: body remains immutable and live until decoder is freed.
            |out| unsafe {
                ffi::ghostty_snapshot_decoder_new_buf(
                    &allocator.abi,
                    out,
                    body.as_ptr(),
                    body.len(),
                )
            },
            ffi::ghostty_snapshot_decoder_free,
        )
        .map_err(fail)?;
        let yes = true;
        let continuation = CONTINUATION_LIMIT;
        for (option, value) in [
            (
                ffi::SNAPSHOT_DECODER_OPT_MAX_CONTINUATION_BYTES,
                (&raw const continuation).cast::<c_void>(),
            ),
            (
                ffi::SNAPSHOT_DECODER_OPT_RETAIN_CONTINUATION,
                (&raw const yes).cast(),
            ),
            (
                ffi::SNAPSHOT_DECODER_OPT_COMPRESS_HISTORY,
                (&raw const yes).cast(),
            ),
        ] {
            // SAFETY: each value uses the option's documented type and is copied.
            check(unsafe { ffi::ghostty_snapshot_decoder_set(decoder.ptr, option, value) })
                .map_err(fail)?;
        }
        let raw = Owned::new(
            // SAFETY: decoder is fresh; success transfers ownership of a complete
            // terminal. Errors leave the output NULL and release partial state.
            |out| unsafe { ffi::ghostty_snapshot_decoder_decode(decoder.ptr, out) },
            ffi::ghostty_terminal_free,
        )
        .map_err(fail)?;
        let mut consumed = 0usize;
        // Optional native paths may recover after an allocation failure. A
        // quota rejection must still reject this restore transaction entirely.
        if allocator.rejected() {
            check(ffi::OUT_OF_MEMORY).map_err(fail)?;
        }
        // SAFETY: SOURCE_OFFSET returns a size_t into consumed.
        let result = unsafe {
            ffi::ghostty_snapshot_decoder_get(
                decoder.ptr,
                ffi::SNAPSHOT_DECODER_DATA_SOURCE_OFFSET,
                (&raw mut consumed).cast(),
            )
        };
        drop(decoder);
        check(result).map_err(fail)?;
        if consumed != body.len() {
            return Err(bad("trailing data after snapshot finish"));
        }
        let mut terminal = Self::wrap(raw, cols, rows).map_err(fail)?;
        terminal._checkpoint_allocator = Some(allocator);
        if terminal.size() != (terminal_grid::Size { cols, rows }) {
            return Err(bad("envelope dimensions disagree with engine state"));
        }
        terminal.cell_width = width;
        terminal.cell_height = height;
        terminal.configure_callbacks()?;
        Ok(terminal)
    }

    /// Atomically replace the display/state only after the whole checkpoint
    /// validates. Preserve this frontend's Option-as-Alt preference.
    pub fn restore_checkpoint(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let mut restored = Self::from_checkpoint(bytes)?;
        restored.option_as_alt = self.option_as_alt;
        *self = restored;
        Ok(())
    }

    /// Update native history limits. Eviction is page-granular, so a terminal
    /// can retain part of one page beyond the requested number of lines.
    pub fn set_scrollback_limits(
        &mut self,
        max_lines: usize,
        max_bytes: usize,
    ) -> Result<(), Error> {
        for (what, option, value) in [
            (
                "scrollback length",
                ffi::TERMINAL_OPT_SCROLLBACK_MAX_LINES,
                &max_lines,
            ),
            (
                "scrollback size",
                ffi::TERMINAL_OPT_SCROLLBACK_MAX_BYTES,
                &max_bytes,
            ),
        ] {
            // SAFETY: these two options take size_t and copy their value.
            check(unsafe {
                ffi::ghostty_terminal_set(self.raw.ptr, option, ptr::from_ref(value).cast())
            })
            .map_err(|source| Error::Configure { what, source })?;
        }
        Ok(())
    }
}
