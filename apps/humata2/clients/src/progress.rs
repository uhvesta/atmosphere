//! PROGRESS: how far a running operation is, as one model every place that
//! shows progress draws (`ui::widgets::progress_line`). The daemon's
//! operations (`pb::Operation`) map to it in `clients/live/src/map.rs`;
//! fakes script it directly.

/// The running step of an operation, as the progress line draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct OpProgress {
    /// The running step's title, as shown: "Cloning acme/api".
    pub title: String,
    /// 0..=1 when the daemon measures the step (`total_units > 0`); `None`
    /// draws the sweep.
    pub fraction: Option<f32>,
    /// What the measured fraction reads as: "42%" or "120 of 512 MB";
    /// empty when unmeasured.
    pub label: String,
    /// When the operation started, unix ms; the line shows nothing for its
    /// first 300 ms and the elapsed time after 10 s.
    pub started_ms: i64,
}

// Feature states that hold an `OpProgress` compare with `Eq`. `fraction` is
// made from integer counts (`done.min(total) / total` with `total > 0`), so it
// is never NaN and equality is reflexive.
impl Eq for OpProgress {}

/// What kind of operation an [`OpUpdate`] is about, so the dialog that
/// started one can pick its own out of the feed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpKind {
    StartSession,
    Fork,
    ContinueInChannel,
    AddRepo,
    Restore,
    Rebuild,
    StartPrReview,
    /// Any kind no dialog follows yet.
    Other,
}

/// A watched operation moved on: its running step, or `None` once nothing
/// runs (it ended, or is between steps).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpUpdate {
    /// The daemon's operation id.
    pub id: String,
    pub kind: OpKind,
    pub progress: Option<OpProgress>,
}
