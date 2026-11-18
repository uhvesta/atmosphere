//! A tab's authoritative screen (D271): one Ghostty terminal per running
//! tab, fed by everything the tab's holder prints, watched or not.
//!
//! It answers the program's terminal queries (cursor position, device
//! attributes) so a program never hangs for want of a viewer, and hands
//! out opaque checkpoints a viewer installs whole, never escape bytes to
//! replay. Each answer is pending from the moment it is handed out until
//! the holder says it was written: the caller saves [`Authority::record`]
//! before writing an answer, and after a restart [`Authority::resume`]
//! answers only queries that never had one. An answer that was in flight
//! is never sent again; the tab's input is uncertain until a resync.
//!
//! It knows nothing about holders, sockets or the wire: the daemon feeds
//! it, writes its answers and carries its checkpoints.

use ghostty_vt::{Event, Terminal};

/// How a tab's screen is sized and bounded.
#[derive(Clone, Debug)]
pub struct Config {
    pub cols: u16,
    pub rows: u16,
    /// Rows of history to keep (dropped a page at a time).
    pub scrollback_lines: usize,
    /// Bytes of history to keep, whatever the row count.
    pub scrollback_bytes: usize,
    /// The largest checkpoint handed out, envelope included.
    pub checkpoint_limit: usize,
}

/// What must survive a restart for answers never to be sent twice. Save
/// it whenever it changes, before writing an answer it lists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplyRecord {
    /// The end offset of the last output that raised an answer. Queries in
    /// output up to here have had theirs.
    pub answered_through: u64,
    /// Answers handed out and not yet confirmed written, by sequence.
    pub pending: Vec<u64>,
    /// An answer was in flight across a restart, so what the program has
    /// read is unknown until the tab resyncs.
    pub uncertain: bool,
}

/// Something the program's output asks of the daemon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Write `bytes` to the program, in order. Save the record first.
    Reply { seq: u64, bytes: Vec<u8> },
    Title(String),
    Pwd(String),
}

/// The screen at `offset`, for a viewer to install.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// The engine and envelope format to send beside the bytes.
    pub engine: &'static str,
    pub bytes: Vec<u8>,
    pub offset: u64,
    pub cols: u16,
    pub rows: u16,
    /// History rows dropped, oldest first, so the checkpoint fits its
    /// limit; 0 when nothing was (D481). The holder still keeps the output.
    pub trimmed_rows: usize,
}

/// A size change, which viewers apply after the output before `at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resized {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u32,
    pub cell_height: u32,
    pub at: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthorityError {
    #[error(
        "The tab's output skipped from byte {expected} to byte {got}, so its screen would be wrong. Rebuild the screen from the holder's kept output; don't feed past the gap."
    )]
    Gap { expected: u64, got: u64 },
    #[error(
        "Couldn't {what} the tab's screen. Keep the holder running and rebuild the screen from its kept output; the program itself is unaffected."
    )]
    Engine {
        what: &'static str,
        #[source]
        source: ghostty_vt::Error,
    },
}

fn engine(what: &'static str) -> impl FnOnce(ghostty_vt::Error) -> AuthorityError {
    move |source| AuthorityError::Engine { what, source }
}

pub struct Authority {
    terminal: Terminal,
    checkpoint_limit: usize,
    history_lines: usize,
    history_bytes: usize,
    /// Offset just past the last byte parsed.
    end: u64,
    /// True when the history starts after byte 0.
    partial: bool,
    next_seq: u64,
    record: ReplyRecord,
    changed: bool,
}

impl Authority {
    /// A blank screen at offset 0, for a holder just started.
    pub fn new(config: &Config) -> Result<Self, AuthorityError> {
        let mut terminal = Terminal::new(config.cols, config.rows, config.scrollback_lines)
            .map_err(engine("create"))?;
        terminal
            .set_scrollback_limits(config.scrollback_lines, config.scrollback_bytes)
            .map_err(engine("bound the history of"))?;
        Ok(Authority {
            terminal,
            checkpoint_limit: config.checkpoint_limit,
            history_lines: config.scrollback_lines,
            history_bytes: config.scrollback_bytes,
            end: 0,
            partial: false,
            next_seq: 1,
            record: ReplyRecord::default(),
            changed: true,
        })
    }

    /// Rebuilds the screen after a restart from the output the holder
    /// kept (`kept` starts at `kept_from`) and the record saved before.
    /// Queries answered before are not answered again; the steps hold
    /// answers to queries that never had one. An answer in flight leaves
    /// input uncertain.
    pub fn resume(
        config: &Config,
        kept_from: u64,
        kept: &[u8],
        record: &ReplyRecord,
    ) -> Result<(Self, Vec<Step>), AuthorityError> {
        let mut authority = Self::new(config)?;
        authority.end = kept_from;
        authority.partial = kept_from > 0;
        authority.record = ReplyRecord {
            answered_through: record.answered_through,
            pending: Vec::new(),
            uncertain: record.uncertain || !record.pending.is_empty(),
        };
        let answered = record
            .answered_through
            .saturating_sub(kept_from)
            .min(kept.len() as u64);
        let split = usize::try_from(answered).unwrap_or(kept.len());
        let (old, new) = kept.split_at(split);
        authority.terminal.feed(old);
        // Answers to these went out before the restart, and the store
        // already has the title and folder they set.
        drop(authority.terminal.drain_events());
        authority.end += old.len() as u64;
        let steps = authority.parse(new);
        authority.changed = true;
        Ok((authority, steps))
    }

    /// Parses output starting at `offset`. Bytes already parsed are
    /// skipped; a gap is refused and nothing past it is taken.
    pub fn feed(&mut self, offset: u64, bytes: &[u8]) -> Result<Vec<Step>, AuthorityError> {
        if offset > self.end {
            return Err(AuthorityError::Gap {
                expected: self.end,
                got: offset,
            });
        }
        let seen = usize::try_from(self.end - offset).unwrap_or(usize::MAX);
        if seen >= bytes.len() {
            return Ok(Vec::new());
        }
        Ok(self.parse(&bytes[seen..]))
    }

    fn parse(&mut self, bytes: &[u8]) -> Vec<Step> {
        if bytes.is_empty() {
            return Vec::new();
        }
        self.terminal.feed(bytes);
        self.end += bytes.len() as u64;
        self.changed = true;
        let mut steps = Vec::new();
        for event in self.terminal.drain_events() {
            match event {
                Event::Reply(bytes) => {
                    let seq = self.next_seq;
                    self.next_seq += 1;
                    self.record.pending.push(seq);
                    self.record.answered_through = self.end;
                    steps.push(Step::Reply { seq, bytes });
                }
                Event::TitleChanged(title) => steps.push(Step::Title(title)),
                Event::PwdChanged(pwd) => steps.push(Step::Pwd(pwd)),
                // The bell, and the clipboard, which the daemon never
                // grants a program on its own: a viewer decides.
                _ => {}
            }
        }
        steps
    }

    /// The holder wrote answer `seq` to the program.
    pub fn written(&mut self, seq: u64) {
        self.record.pending.retain(|pending| *pending != seq);
    }

    /// What to save so a restart answers nothing twice.
    pub fn record(&self) -> ReplyRecord {
        self.record.clone()
    }

    pub fn input_uncertain(&self) -> bool {
        self.record.uncertain
    }

    /// Viewers have a fresh screen and know input may have been lost.
    pub fn resynced(&mut self) {
        self.record.uncertain = false;
    }

    /// The history starts after the holder's first byte, as when the
    /// holder had trimmed output before a restart.
    pub fn history_partial(&self) -> bool {
        self.partial
    }

    /// The offset just past the last byte parsed.
    pub fn end(&self) -> u64 {
        self.end
    }

    /// The text of the screen's active area as a person reads it now, rows
    /// joined by line breaks; history above it is left out.
    pub fn screen_text(&self) -> Result<String, AuthorityError> {
        let size = self.terminal.size();
        let total = u32::try_from(self.terminal.total_rows()).unwrap_or(u32::MAX);
        let rows = u32::from(size.rows.max(1));
        let first = total.saturating_sub(rows);
        let last = total.saturating_sub(1).max(first);
        self.terminal
            .selection_text(
                ghostty_vt::Point { x: 0, y: first },
                ghostty_vt::Point {
                    x: size.cols.saturating_sub(1),
                    y: last,
                },
            )
            .map_err(engine("read the text of"))
    }

    /// Resizes the screen. Send the same size to the holder, then the
    /// returned change to every viewer, in order with the output.
    pub fn resize(
        &mut self,
        cols: u16,
        rows: u16,
        cell_width: u32,
        cell_height: u32,
    ) -> Result<Resized, AuthorityError> {
        self.terminal
            .resize_with_cell_size(cols, rows, cell_width, cell_height)
            .map_err(engine("resize"))?;
        self.changed = true;
        Ok(Resized {
            cols,
            rows,
            cell_width,
            cell_height,
            at: self.end,
        })
    }

    /// Whether the screen changed since the last checkpoint, so an
    /// unchanged one can be reused.
    pub fn changed(&self) -> bool {
        self.changed
    }

    /// The screen as it is now, for a viewer to install.
    pub fn snapshot(&mut self) -> Result<Snapshot, AuthorityError> {
        // Over the limit, the oldest history goes instead of the snapshot
        // failing; only a screen that doesn't fit even bare is an error.
        let (bytes, trimmed_rows) = self
            .terminal
            .checkpoint_trimmed(self.checkpoint_limit, self.history_lines, self.history_bytes)
            .map_err(engine("checkpoint"))?;
        let size = self.terminal.size();
        self.changed = false;
        Ok(Snapshot {
            engine: ghostty_vt::CHECKPOINT_FORMAT,
            bytes,
            offset: self.end,
            cols: size.cols,
            rows: size.rows,
            trimmed_rows,
        })
    }
}
