//! The compression settings a daemon reads: Settings › Advanced, globally
//! and per machine (TR-11). Until Settings has a page for it, it is a file
//! in the daemon's state folder:
//!
//! ```toml
//! compression = "auto"          # auto, lz4, zstd or off
//!
//! [machines.ma_0123456789abcdef]
//! compression = "zstd"
//! ```

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::Path;

use serde::Deserialize;

use crate::error::SettingsError;
use crate::policy::Preference;

/// The settings file's name in the daemon's state folder.
pub const SETTINGS_FILE: &str = "compression.toml";

/// The compression preference for every machine, and for some on their own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default)]
    compression: Preference,
    #[serde(default)]
    machines: BTreeMap<String, MachineSettings>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct MachineSettings {
    compression: Option<Preference>,
}

impl Settings {
    /// Reads the settings at `path`. No file means Auto everywhere.
    pub fn load(path: &Path) -> Result<Settings, SettingsError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Settings::default()),
            Err(source) => {
                return Err(SettingsError::Read {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        toml::from_str(&text).map_err(|source| SettingsError::Parse {
            path: path.to_owned(),
            source,
        })
    }

    /// The preference for streams on this machine's own sockets and for
    /// any machine without one of its own.
    pub fn global(&self) -> Preference {
        self.compression
    }

    /// The preference for streams with `machine`: its own, else the global
    /// one.
    pub fn for_machine(&self, machine: &str) -> Preference {
        self.machines
            .get(machine)
            .and_then(|settings| settings.compression)
            .unwrap_or(self.compression)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(text: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let path = folder.path().join(SETTINGS_FILE);
        std::fs::write(&path, text).expect("the settings file");
        (folder, path)
    }

    #[test]
    fn no_file_is_auto_everywhere() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let settings = Settings::load(&folder.path().join(SETTINGS_FILE)).expect("defaults");
        assert_eq!(settings.global(), Preference::Auto);
        assert_eq!(settings.for_machine("ma_1"), Preference::Auto);
    }

    #[test]
    fn a_machine_of_its_own_overrides_the_global_preference() {
        let (_folder, path) = write(
            "compression = \"off\"\n[machines.ma_slow]\ncompression = \"zstd\"\n[machines.ma_plain]\n",
        );
        let settings = Settings::load(&path).expect("valid settings");
        assert_eq!(settings.global(), Preference::Off);
        assert_eq!(settings.for_machine("ma_slow"), Preference::Zstd);
        assert_eq!(settings.for_machine("ma_plain"), Preference::Off);
        assert_eq!(settings.for_machine("ma_other"), Preference::Off);
    }

    #[test]
    fn every_preference_parses() {
        for (word, preference) in [
            ("auto", Preference::Auto),
            ("lz4", Preference::Lz4),
            ("zstd", Preference::Zstd),
            ("off", Preference::Off),
        ] {
            let (_folder, path) = write(&format!("compression = \"{word}\"\n"));
            assert_eq!(Settings::load(&path).expect("valid settings").global(), preference);
        }
    }

    #[test]
    fn an_unknown_codec_or_key_is_an_error() {
        let (_folder, path) = write("compression = \"brotli\"\n");
        assert!(matches!(Settings::load(&path), Err(SettingsError::Parse { .. })));
        let (_folder, path) = write("compresion = \"lz4\"\n");
        assert!(matches!(Settings::load(&path), Err(SettingsError::Parse { .. })));
    }
}
