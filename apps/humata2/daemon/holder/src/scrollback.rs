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
        match self.runs.back_mut() {
            Some(run) if run.end() == self.end => run.bytes.extend(data),
            _ => self.runs.push_back(Run {
                start: self.end,
                bytes: data.iter().copied().collect(),
            }),
        }
        self.end += data.len() as u64;
        self.kept += data.len();
        while self.kept > self.limit {
            let over = self.kept - self.limit;
            let Some(front) = self.runs.front_mut() else {
                break;
            };
            if front.bytes.len() <= over {
                self.kept -= front.bytes.len();
                self.runs.pop_front();
            } else {
                front.bytes.drain(..over);
                front.start += over as u64;
                self.kept -= over;
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
