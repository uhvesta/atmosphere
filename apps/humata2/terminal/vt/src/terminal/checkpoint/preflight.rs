//! Bound the pinned wire format's allocations that bypass its GPA on native.
//! Ghostty remains the semantic/CRC decoder; this does no terminal mutation.
use super::validate_grid;
use crate::Error;

const PAGE_BUDGET: u64 = 256 * 1024 * 1024;
const MAX_RECORDS: usize = 65_536;

fn bad(reason: &'static str) -> Error {
    Error::CheckpointHeader { reason }
}
fn word(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}
fn dword(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

pub(super) fn validate(bytes: &[u8], cols: u16, rows: u16) -> Result<(), Error> {
    if bytes.len() < 10 || &bytes[..8] != b"GHOSTSNP" || word(bytes, 8) != 1 {
        return Err(bad("unsupported Ghostty snapshot envelope"));
    }
    let mut at = 10usize;
    let mut count = 0usize;
    let mut terminal_seen = false;
    let mut page_budget = 0u64;
    while at < bytes.len() {
        count += 1;
        if count > MAX_RECORDS {
            return Err(bad("too many snapshot records"));
        }
        let header = bytes
            .get(at..at + 10)
            .ok_or_else(|| bad("truncated snapshot record header"))?;
        let tag = word(header, 0);
        let len = dword(header, 2) as usize;
        let start = at + 10;
        let end = start
            .checked_add(len)
            .ok_or_else(|| bad("snapshot record length overflow"))?;
        let payload = bytes
            .get(start..end)
            .ok_or_else(|| bad("truncated snapshot record payload"))?;
        match tag {
            1 => {
                if terminal_seen || payload.len() < 4 {
                    return Err(bad("invalid terminal record"));
                }
                terminal_seen = true;
                let actual_cols = word(payload, 0);
                let actual_rows = word(payload, 2);
                validate_grid(actual_cols, actual_rows)?;
                if actual_cols != cols || actual_rows != rows {
                    return Err(bad("envelope dimensions disagree with terminal record"));
                }
            }
            3 => {
                if payload.len() < 20 {
                    return Err(bad("truncated page capacity header"));
                }
                let columns = u64::from(word(payload, 0));
                let page_rows = u64::from(word(payload, 2));
                if columns == 0 || page_rows == 0 || columns > 4096 {
                    return Err(bad("unsupported page dimensions"));
                }
                // A conservative capacity estimate, not an RSS measurement.
                // It overbudgets cells, rows, table load-factor/rounding, and
                // bitmap-backed strings/graphemes, plus 1MiB per page for
                // native pool/alignment overhead. Check every hint before the
                // C decoder can perform pointer-size arithmetic or mmap.
                page_budget += columns * page_rows * 32
                    + page_rows * 64
                    + u64::from(word(payload, 8)) * 256
                    + u64::from(word(payload, 10)) * 256
                    + u64::from(dword(payload, 12)) * 32
                    + u64::from(dword(payload, 16)) * 8
                    + 1024 * 1024;
                if page_budget > PAGE_BUDGET {
                    return Err(Error::CheckpointPageLimit {
                        requested: page_budget,
                        limit: PAGE_BUDGET,
                    });
                }
            }
            _ => {}
        }
        at = end;
    }
    if !terminal_seen {
        return Err(bad("missing terminal record"));
    }
    Ok(())
}
