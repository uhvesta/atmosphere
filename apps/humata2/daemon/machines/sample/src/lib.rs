//! The commented sample `~/.humata2/config/machine-scripts.toml` (D680),
//! shared by the daemon's rules parser and the app's Edit rules starter.

/// Every line is a comment, so no rule runs until the owner writes one.
pub const SAMPLE_RULES: &str = include_str!("../machine-scripts.toml");
