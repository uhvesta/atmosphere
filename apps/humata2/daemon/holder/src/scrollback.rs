use std::collections::VecDeque;

/// The newest output, up to a byte limit, addressed by offsets that count
/// every byte the program has printed. A holder that took over after
/// another was killed starts its program's output past the old one's, so
/// what's kept can have gaps: runs of offsets no byte was kept for.
pub(crate) struct Scrollback {
    runs: VecDeque<Run>,
    kept: usize,
    end: u64,
    limit: usize,
}

/// Bytes kept from `start` on, with no gap among them.
struct Run {
    start: u64,
    bytes: VecDeque<u8>,
}

impl Run {
    fn end(&self) -> u64 {
        self.start + self.bytes.len() as u64
    }
}

impl Scrollback {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            runs: VecDeque::new(),
            kept: 0,
            end: 0,
            limit: limit.max(1),
        }
    }

    /// A scrollback as it was: `runs` (start offset and bytes, oldest
    /// first) kept up to `limit`, the next byte at offset `end`.
    pub(crate) fn restore(limit: usize, end: u64, runs: Vec<(u64, Vec<u8>)>) -> Self {
        let mut kept = 0;
        let mut rebuilt = VecDeque::new();
        for (start, bytes) in runs {
            kept += bytes.len();
            rebuilt.push_back(Run {
                start,
                bytes: VecDeque::from(bytes),
            });
        }
        Self {
            runs: rebuilt,
            kept,
            end,
            limit: limit.max(1),
        }
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit
    }

    pub(crate) fn earliest(&self) -> u64 {
        self.runs.front().map_or(self.end, |run| run.start)
    }

    pub(crate) fn end(&self) -> u64 {
        self.end
    }

    /// Moves the next byte's offset on to `offset`, leaving a gap. Never
    /// moves it back.
    pub(crate) fn skip_to(&mut self, offset: u64) {
        self.end = self.end.max(offset);
    }

    pub(crate) fn push(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        // Only the newest `limit` bytes of `data` can be kept, and room
        // is made before they're added: kept output never grows past the
        // limit, so neither does the memory holding it (FAST, D789). A
        // full scrollback that grew first and trimmed after doubled its
        // buffer, to twice the limit, for every busy tab.
        let skipped = data.len().saturating_sub(self.limit);
        let data = &data[skipped..];
        self.end += skipped as u64;
        self.drop_oldest((self.kept + data.len()).saturating_sub(self.limit));
        let limit = self.limit;
        match self.runs.back_mut() {
            Some(run) if run.end() == self.end => {
                let needed = run.bytes.len() + data.len();
                if needed > run.bytes.capacity() {
                    // Grows by doubling, as a VecDeque does, but never
                    // past the limit.
                    let wanted = needed.max(run.bytes.capacity() * 2).min(limit);
                    run.bytes.reserve_exact(wanted - run.bytes.len());
                }
                run.bytes.extend(data);
            }
            _ => self.runs.push_back(Run {
                start: self.end,
                bytes: data.iter().copied().collect(),
            }),
        }
        self.end += data.len() as u64;
        self.kept += data.len();
    }

    /// Drops the oldest `over` bytes kept, whole runs first.
    fn drop_oldest(&mut self, mut over: usize) {
        while over > 0 {
            let Some(front) = self.runs.front_mut() else {
                break;
            };
            if front.bytes.len() <= over {
                over -= front.bytes.len();
                self.kept -= front.bytes.len();
                self.runs.pop_front();
            } else {
                front.bytes.drain(..over);
                front.start += over as u64;
                self.kept -= over;
                over = 0;
            }
        }
    }

    /// Calls `piece` with each stretch of kept bytes, oldest first, and
    /// the offset it starts at, without copying them.
    pub(crate) fn each_piece(&self, mut piece: impl FnMut(u64, &[u8])) {
        for run in &self.runs {
            let (front, back) = run.bytes.as_slices();
            piece(run.start, front);
            if !back.is_empty() {
                piece(run.start + front.len() as u64, back);
            }
        }
    }

    /// The kept bytes from `from` on, as runs with the offset each starts
    /// at.
    pub(crate) fn since(&self, from: u64) -> Vec<(u64, Vec<u8>)> {
        self.runs
            .iter()
            .filter(|run| run.end() > from)
            .map(|run| {
                let start = from.max(run.start);
                let skip = usize::try_from(start - run.start).unwrap_or(usize::MAX);
                (start, run.bytes.range(skip..).copied().collect())
            })
            .collect()
    }
}

#[cfg(test)]
impl Scrollback {
    /// The bytes of memory the kept output holds, used or not.
    fn held(&self) -> usize {
        self.runs.iter().map(|run| run.bytes.capacity()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::Scrollback;

    const LIMIT: usize = 8 << 20;

    /// Streams three limits' worth of output in `chunk`-byte pieces, each
    /// byte its offset's low byte, and returns the scrollback.
    fn streamed(first: usize, chunk: usize) -> Scrollback {
        let mut kept = Scrollback::new(LIMIT);
        let mut offset = 0usize;
        let mut len = first;
        while offset < 3 * LIMIT {
            let data: Vec<u8> = (offset..offset + len).map(|at| at as u8).collect();
            kept.push(&data);
            offset += len;
            len = chunk;
        }
        kept
    }

    #[test]
    fn a_full_scrollback_holds_no_more_memory_than_its_limit() {
        for (first, chunk) in [(16 << 10, 16 << 10), (1000, 4096), (300, 300), (64 << 10, 64 << 10)] {
            let kept = streamed(first, chunk);
            println!(
                "scrollback of {LIMIT} bytes after {} bytes in {chunk}-byte reads (first {first}): holds {} bytes",
                kept.end(),
                kept.held()
            );
            assert!(
                kept.held() <= LIMIT,
                "{chunk}-byte reads: holds {} bytes for a {LIMIT}-byte limit",
                kept.held()
            );
        }
    }

    #[test]
    fn it_keeps_the_newest_limit_bytes() {
        let kept = streamed(1000, 4096);
        assert_eq!(kept.earliest(), kept.end() - LIMIT as u64);
        let runs = kept.since(0);
        assert_eq!(runs.len(), 1);
        let (start, bytes) = &runs[0];
        assert_eq!(*start, kept.earliest());
        assert_eq!(bytes.len(), LIMIT);
        assert!(bytes.iter().zip(*start..).all(|(byte, at)| *byte == at as u8));
    }

    #[test]
    fn a_piece_longer_than_the_limit_keeps_its_tail() {
        let mut kept = Scrollback::new(10);
        kept.push(b"abc");
        kept.push(b"0123456789abcdef");
        assert_eq!(kept.end(), 19);
        assert_eq!(kept.since(0), vec![(9, b"6789abcdef".to_vec())]);
        assert!(kept.held() <= 10);
    }

    #[test]
    fn a_gap_starts_a_new_run_and_both_are_kept_up_to_the_limit() {
        let mut kept = Scrollback::new(10);
        kept.push(b"abcd");
        kept.skip_to(100);
        kept.push(b"wxyz");
        assert_eq!(kept.since(0), vec![(0, b"abcd".to_vec()), (100, b"wxyz".to_vec())]);
        kept.push(b"12345");
        assert_eq!(kept.since(0), vec![(3, b"d".to_vec()), (100, b"wxyz12345".to_vec())]);
        assert!(kept.held() <= 10 + 4, "the older run keeps its own small buffer");
    }
}
