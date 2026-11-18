//! The names machines are shown by, for the errors that name a machine
//! (D778). An error carries the
//! machine's id, which the app acts on; what the person reads is the name
//! the machine list gave it ("This Mac", "devbox-1"), never the raw `ma_`
//! id. A machine the app hasn't listed yet reads as "That machine".

use std::collections::HashMap;
use std::sync::{OnceLock, PoisonError, RwLock};

use crate::model::MachineId;

/// What a machine whose name the app doesn't know yet is called.
const UNKNOWN: &str = "That machine";

fn names() -> &'static RwLock<HashMap<MachineId, String>> {
    static NAMES: OnceLock<RwLock<HashMap<MachineId, String>>> = OnceLock::new();
    NAMES.get_or_init(RwLock::default)
}

/// Remembers that `id` is shown as `name`. An empty name is ignored, so a
/// listing that left the name out never hides the one known.
pub fn remember(id: &MachineId, name: &str) {
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    let mut names = names().write().unwrap_or_else(PoisonError::into_inner);
    if names.get(id).map(String::as_str) != Some(name) {
        names.insert(id.clone(), name.to_owned());
    }
}

/// The name `id` is shown by: the one remembered, else "That machine".
#[must_use]
pub fn name_of(id: &MachineId) -> String {
    names()
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(id)
        .cloned()
        .unwrap_or_else(|| UNKNOWN.to_owned())
}
