//! Global `lore` TOML configuration, kill-switch (`SW-1`, `SW-2`, `SW-4`, `SW-5`),
//! and retention / GC thresholds (`GC-1`..`GC-6`).

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use lore_proto::{LoreError, Result};
use serde::{Deserialize, Serialize};

/// Milliseconds in one 24-hour day.
pub const MS_PER_DAY: i64 = 24 * 60 * 60 * 1000;

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// `[memory]` section controlling the global kill-switch (`SW-1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    /// Global enablement flag. When `false`, all agent commands and MCP tools
    /// refuse immediately without touching storage.
    pub enabled: bool,
    /// Optional human-supplied reason surfaced in refusal messages and `lore status`.
    pub disabled_reason: Option<String>,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            disabled_reason: None,
        }
    }
}

/// `[retention]` section governing `(status, age)` archival and GC purges.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetentionConfig {
    /// Days before `SUPERSEDED` / `INVALIDATED` claims move from Tier-0 Hot to Tier-1 Cold.
    pub hot_inactive_days: u64,
    /// Days before unpinned non-`AXIOM`/non-`DECISION` `ACTIVE` / `EXPIRED` claims
    /// move from Tier-0 Hot to Tier-1 Cold.
    pub hot_dormant_days: u64,
    /// Days before non-active `HANDOFF` claims are purged in Tier-2 GC.
    pub handoff_purge_days: u64,
    /// Days before `RETRACTED` claims are purged in Tier-2 GC.
    pub retracted_purge_days: u64,
    /// Days before uncorroborated `QUARANTINED` claims are purged in Tier-2 GC.
    pub quarantined_purge_days: u64,
    /// Days before `claim_exposures` rows are folded into counters and pruned.
    pub exposure_fold_days: u64,
    /// Number of cold-eligible or appended write events before lazy GC triggers.
    pub gc_write_watermark: u64,
    /// Total hot segment byte watermark before lazy GC triggers (default 16 MiB).
    pub gc_hot_bytes_watermark: u64,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            hot_inactive_days: 7,
            hot_dormant_days: 90,
            handoff_purge_days: 14,
            retracted_purge_days: 30,
            quarantined_purge_days: 14,
            exposure_fold_days: 7,
            gc_write_watermark: 512,
            gc_hot_bytes_watermark: 16 * 1024 * 1024,
        }
    }
}

/// Top-level `config.toml` schema stored at `<root>/config.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoreConfig {
    pub memory: MemoryConfig,
    pub retention: RetentionConfig,
}

/// Returns `<root>/config.toml`.
pub fn config_path(root: &Path) -> PathBuf {
    root.join("config.toml")
}

/// Loads `config.toml` from `root`, returning defaults if the file does not exist.
pub fn load_config(root: &Path) -> Result<LoreConfig> {
    let path = config_path(root);
    match fs::read_to_string(&path) {
        Ok(raw) => toml::from_str::<LoreConfig>(&raw).map_err(|err| LoreError::Io {
            path: path.clone(),
            source: io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to parse lore config TOML: {err}"),
            ),
        }),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(LoreConfig::default()),
        Err(source) => Err(LoreError::Io { path, source }),
    }
}

/// Atomically writes `config` to `<root>/config.toml` via temp file + `sync_all` + rename.
pub fn save_config(root: &Path, config: &LoreConfig) -> Result<()> {
    fs::create_dir_all(root).map_err(|source| LoreError::Io {
        path: root.to_path_buf(),
        source,
    })?;

    let path = config_path(root);
    let rendered = toml::to_string_pretty(config).map_err(|err| LoreError::Io {
        path: path.clone(),
        source: io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to serialize lore config TOML: {err}"),
        ),
    })?;

    let nonce = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_path = root.join(format!("config.toml.tmp.{}.{nonce}", std::process::id()));

    let write_res = (|| -> io::Result<()> {
        let mut file = File::create(&tmp_path)?;
        file.write_all(rendered.as_bytes())?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(source) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(LoreError::Io {
            path: tmp_path,
            source,
        });
    }

    fs::rename(&tmp_path, &path).map_err(|source| {
        let _ = fs::remove_file(&tmp_path);
        LoreError::Io { path, source }
    })?;

    Ok(())
}

/// Enforces `SW-4`: returns `Err(LoreError::Disabled { reason })` when `memory.enabled == false`.
pub fn ensure_enabled(config: &LoreConfig) -> Result<()> {
    if config.memory.enabled {
        Ok(())
    } else {
        Err(LoreError::Disabled {
            reason: config.memory.disabled_reason.clone(),
        })
    }
}
