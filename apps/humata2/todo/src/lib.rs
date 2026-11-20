//! The to-do rules: one ranked list replaces boards. Pure functions, no IO, no clock.

mod day;
mod due;
mod filter;
mod migrate;
mod model;
mod quick_add;
mod rank;

pub use day::{Day, ParseDayError, Weekday, parse_day};
pub use due::{DueChip, Tone, chip, chip_for, fuzzy_chip};
pub use filter::{
    ChannelFilter, DueFilter, Filter, Only, SessionFilter, apply, channel_counts, left_out,
};
pub use migrate::{OldItem, Summary, migrate, migrate_with_summary};
pub use model::{
    Ask, AskDetail, AskKind, BlockerGone, CheckpointState, CloseReadiness, Effort, Evidence,
    ExternalBlock, FuzzyDue, HeldBackBy, Impact, LifecyclePrompt, Need, OldColumn, OwnerKind,
    Review, RunEntry, RunLink, SessionSignal, SortBy, SourceAnchor, Start, State, Step, StuckAgent,
    Todo, TodoComment, TodoOwner, WatchedPr, WordDiffKind, WordDiffSpan, format_cost_usd,
    word_diff,
};
pub use quick_add::{Draft, parse};
pub use rank::{Ranked, Reason, Row, header, rank, rank_with};
