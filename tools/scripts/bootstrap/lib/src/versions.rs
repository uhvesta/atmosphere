use std::path::Path;

use serde::Deserialize;

use crate::{Error, read_to_string};

/// The versions to pin. Every field is required, so a typo fails loudly.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Versions {
    pub bazelisk: String,
}

impl Versions {
    pub fn read(dir: &Path) -> Result<Self, Error> {
        let path = dir.join("versions.toml");
        let text = read_to_string(&path)?;
        toml::from_str(&text).map_err(|source| Error::InvalidVersions { path, source })
    }
}
