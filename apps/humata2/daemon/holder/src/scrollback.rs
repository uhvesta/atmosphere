use std::collections::VecDeque;

/// The newest output, up to a byte limit, addressed by offsets that count
/// every byte the program has printed.
pub(crate) struct Scrollback {
    bytes: VecDeque<u8>,
    earliest: u64,
    limit: usize,
}

impl Scrollback {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            bytes: VecDeque::new(),
            earliest: 0,
            limit: limit.max(1),
        }
    }

    pub(crate) fn earliest(&self) -> u64 {
        self.earliest
    }

    pub(crate) fn end(&self) -> u64 {
        self.earliest + self.bytes.len() as u64
    }

    pub(crate) fn push(&mut self, data: &[u8]) {
        if data.len() >= self.limit {
            self.earliest = self.end() + (data.len() - self.limit) as u64;
            self.bytes.clear();
            self.bytes.extend(&data[data.len() - self.limit..]);
            return;
        }
        self.bytes.extend(data);
        let over = self.bytes.len().saturating_sub(self.limit);
        self.bytes.drain(..over);
        self.earliest += over as u64;
    }

    /// The kept bytes from `from`, clamped to what's kept, and the offset
    /// they start at.
    pub(crate) fn since(&self, from: u64) -> (u64, Vec<u8>) {
        let start = from.clamp(self.earliest, self.end());
        let skip = usize::try_from(start - self.earliest).unwrap_or(usize::MAX);
        (start, self.bytes.range(skip..).copied().collect())
    }
}
