//! Shared start-fresh handover types (`Handover`, `Source`, `retype_pending`).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Transcript,
    Scrollback,
    None,
}

#[derive(Clone, Debug)]
pub struct Handover {
    pub source: Source,
    pub summary: String,
}

pub fn retype_pending(items: &[String]) -> Vec<String> {
    items
        .iter()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}
