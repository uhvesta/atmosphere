//! The generated JSON catalogs, keyed by version.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::Error;
use crate::version::Version;

pub(crate) type Catalog<T> = BTreeMap<Version, T>;

/// The entries in `path`, or none when it doesn't exist yet or `refresh` asks
/// to rebuild it.
pub(crate) fn read<T: DeserializeOwned>(path: &Path, refresh: bool) -> Result<Catalog<T>, Error> {
    if refresh || !path.exists() {
        return Ok(Catalog::new());
    }
    let bytes = fs::read(path).map_err(|source| bootstrap::Error::Read {
        path: path.to_owned(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| Error::InvalidCatalog {
        path: path.to_owned(),
        source,
    })
}

/// Writes `entries` in version order, two-space indented, so the file is
/// byte-identical whichever machine regenerates it. The file is replaced in
/// one step, so an interrupted run never leaves half of it.
pub(crate) fn write<T: Serialize>(path: &Path, entries: &Catalog<T>) -> Result<(), Error> {
    let temporary = path.with_extension("json.tmp");
    let write_error = |path: &Path| {
        let path = path.to_owned();
        move |source| bootstrap::Error::Write { path, source }
    };
    let mut text =
        serde_json::to_string_pretty(entries).map_err(|error| write_error(path)(error.into()))?;
    text.push('\n');
    fs::write(&temporary, text).map_err(write_error(&temporary))?;
    fs::rename(&temporary, path).map_err(write_error(path))?;
    Ok(())
}
