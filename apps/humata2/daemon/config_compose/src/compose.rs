//! Native per-session agent config materializer, instant non-destructive
//! rage-quit switch (`disable_sync`), and sanitized FUSE config view generator
//! (`sanitized_view`).

use crate::model::{
    AgentKind, ConfigFormat, EffectiveSyncPolicy, SecretDenylist, SkillsMode, SyncCategory,
    SyncConfig,
};
use crate::rewrite::{
    ProtectedHumataMcp, RewriteContext, RewritePipeline, RewriteViolation,
    enforce_protected_humata_mcp, json_to_toml, toml_to_json,
};
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Errors returned during native config composition or sanitized view generation.
#[derive(Debug, Error)]
pub enum ComposeError {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse JSON config {path}: {source}")]
    InvalidJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("failed to parse TOML config {path}: {source}")]
    InvalidToml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("failed to serialize TOML config {path}: {message}")]
    TomlSerialize { path: PathBuf, message: String },
}

/// Environment variables to set when launching an agent in its composed native session home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEnvPlan {
    pub agent: AgentKind,
    pub config_dir: PathBuf,
    pub env_vars: BTreeMap<String, String>,
}

/// Summary returned by `materialize_session`, `disable_sync`, and `enable_sync`.
#[derive(Debug, Clone)]
pub struct MaterializeReport {
    pub session_id: String,
    pub agent: AgentKind,
    pub session_config_dir: PathBuf,
    pub effective_policy: EffectiveSyncPolicy,
    pub env_plan: SessionEnvPlan,
    pub auth_passthrough_links: Vec<PathBuf>,
    pub rewrite_violations: Vec<RewriteViolation>,
    /// Always `false`: config sync & rage-quit atomically update native pointers
    /// in place without restarting any terminal holder (`LIVEHOLDER`).
    pub holder_restart_required: bool,
}

/// Secret-scrubbed FUSE view of `<machine-root>/.sessions/<id>/config/<agent>`.
#[derive(Debug, Clone, Default)]
pub struct SanitizedConfigView {
    /// Non-secret files ready to serve through `s/<id>/config/<agent>`, keyed by
    /// relative path inside the session config directory.
    pub readable_files: BTreeMap<PathBuf, Vec<u8>>,
    /// Relative paths classified as `Secret` that FUSE MUST reject with
    /// `CredentialOutsideMount` (`EACCES`).
    pub denied_secret_paths: BTreeSet<PathBuf>,
}

impl SanitizedConfigView {
    pub fn get_file(&self, rel_path: impl AsRef<Path>) -> Option<&[u8]> {
        self.readable_files
            .get(rel_path.as_ref())
            .map(Vec::as_slice)
    }

    pub fn is_denied(&self, rel_path: impl AsRef<Path>) -> bool {
        self.denied_secret_paths.contains(rel_path.as_ref())
    }
}

/// Native per-session config composer for a machine.
///
/// Guarantees:
/// - Never mutates anything under `local_home` (real `~/.claude`, `~/.codex`,
///   `~/.copilot`, `~/.gemini`).
/// - Materializes per-session config into `<machine_root>/.sessions/<id>/config/<agent>/`.
/// - Wires machine-local auth pass-through symlinks without copying secrets.
/// - Supports instant non-destructive rage-quit (`disable_sync`) with zero holder restarts.
#[derive(Debug, Clone)]
pub struct ConfigComposer {
    pub machine_root: PathBuf,
    pub local_home: PathBuf,
    pub synced_bundle_root: Option<PathBuf>,
    pub sync_config: SyncConfig,
    pub denylist: SecretDenylist,
    pub rewrite_pipeline: RewritePipeline,
    pub protected_humata_mcp: ProtectedHumataMcp,
    pub machine_id: String,
    pub machine_name: String,
    pub os: String,
    pub arch: String,
    pub local_env: BTreeMap<String, String>,
}

impl ConfigComposer {
    pub fn new(
        machine_root: impl Into<PathBuf>,
        local_home: impl Into<PathBuf>,
        machine_name: impl Into<String>,
        protected_humata_mcp: ProtectedHumataMcp,
    ) -> Self {
        let name = machine_name.into();
        Self {
            machine_root: machine_root.into(),
            local_home: local_home.into(),
            synced_bundle_root: None,
            sync_config: SyncConfig::default(),
            denylist: SecretDenylist::default(),
            rewrite_pipeline: RewritePipeline::default(),
            protected_humata_mcp,
            machine_id: format!("id-{name}"),
            machine_name: name,
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            local_env: BTreeMap::new(),
        }
    }

    /// Returns `<machine-root>/.sessions/<session_id>/config/<agent>`.
    pub fn session_config_dir(&self, session_id: &str, agent: AgentKind) -> PathBuf {
        self.machine_root
            .join(".sessions")
            .join(session_id)
            .join("config")
            .join(agent.as_str())
    }

    /// Returns `<local_home>/<agent.default_home_rel_path()>`.
    pub fn local_agent_home(&self, agent: AgentKind) -> PathBuf {
        self.local_home.join(agent.default_home_rel_path())
    }

    /// Returns `<synced_bundle_root>/<agent>` if a synced host bundle is present.
    pub fn synced_agent_dir(&self, agent: AgentKind) -> Option<PathBuf> {
        self.synced_bundle_root
            .as_ref()
            .map(|root| root.join(agent.as_str()))
    }

    /// Materializes `<machine-root>/.sessions/<session_id>/config/<agent>/`
    /// natively with atomic writes, local auth pass-through, `[sync]` policy
    /// resolution, and the 3-stage rewrite pipeline.
    pub fn materialize_session(
        &self,
        session_id: &str,
        agent: AgentKind,
    ) -> Result<MaterializeReport, ComposeError> {
        let session_dir = self.session_config_dir(session_id, agent);
        fs::create_dir_all(&session_dir).map_err(|source| ComposeError::Io {
            path: session_dir.clone(),
            source,
        })?;

        let spec = agent.spec();
        let policy = self.sync_config.effective_for_machine(&self.machine_name);
        let local_agent_dir = self.local_agent_home(agent);
        let synced_agent_dir = self.synced_agent_dir(agent);
        let mut rewrite_violations = Vec::new();

        // 1. Machine-local auth pass-through symlinks (NEVER copied, NEVER mutates local_home).
        let mut auth_links = Vec::new();
        for rel_auth in spec.auth_passthrough_files {
            let src = local_agent_dir.join(rel_auth);
            if src.exists() {
                let dst = session_dir.join(rel_auth);
                atomic_symlink(&src, &dst)?;
                auth_links.push(dst);
            }
        }
        for (home_rel_src, session_rel_dst) in spec.extra_home_auth_sources {
            let src = self.local_home.join(home_rel_src);
            if src.exists() {
                let dst = session_dir.join(session_rel_dst);
                // Do not overwrite if already linked from agent's primary dir.
                if fs::symlink_metadata(&dst).is_err() {
                    atomic_symlink(&src, &dst)?;
                    auth_links.push(dst);
                }
            }
        }

        // 2. Skills category.
        self.compose_skills(
            &session_dir,
            &local_agent_dir,
            synced_agent_dir.as_deref(),
            spec.skills_entries,
            policy,
        )?;

        // 3. Instructions category.
        self.compose_instructions(
            &session_dir,
            &local_agent_dir,
            synced_agent_dir.as_deref(),
            spec.instructions_files,
            policy,
        )?;

        // 4. MCP & Settings categories (handles combined files like Codex `config.toml`).
        let ctx = RewriteContext {
            machine_id: self.machine_id.clone(),
            machine_name: self.machine_name.clone(),
            os: self.os.clone(),
            arch: self.arch.clone(),
            home_dir: self.local_home.clone(),
            agent,
            local_env: self.local_env.clone(),
        };

        self.compose_mcp_and_settings(
            &session_dir,
            &local_agent_dir,
            synced_agent_dir.as_deref(),
            agent,
            policy,
            &ctx,
            &mut rewrite_violations,
        )?;

        // 5. Build launch environment plan.
        let mut env_vars = BTreeMap::new();
        env_vars.insert(
            agent.config_env_var().to_string(),
            session_dir.to_string_lossy().into_owned(),
        );
        if agent == AgentKind::Claude && self.os == "macos" {
            // Allows Claude Code to read/refresh macOS Login Keychain natively.
            env_vars.insert("CLAUDE_SECURESTORAGE_CONFIG_DIR".to_string(), String::new());
        }

        Ok(MaterializeReport {
            session_id: session_id.to_string(),
            agent,
            session_config_dir: session_dir.clone(),
            effective_policy: policy,
            env_plan: SessionEnvPlan {
                agent,
                config_dir: session_dir,
                env_vars,
            },
            auth_passthrough_links: auth_links,
            rewrite_violations,
            holder_restart_required: false,
        })
    }

    /// Instant non-destructive **rage-quit switch**: disables host config sync
    /// globally and atomically switches the session's composed config pointers
    /// back to machine-local `~/.<agent>` without mutating `local_home` and
    /// without restarting any terminal holder.
    pub fn disable_sync(
        &mut self,
        session_id: &str,
        agent: AgentKind,
    ) -> Result<MaterializeReport, ComposeError> {
        self.sync_config.rage_quit();
        self.materialize_session(session_id, agent)
    }

    /// Re-enables host config sync globally and re-materializes from cached bundle.
    pub fn enable_sync(
        &mut self,
        session_id: &str,
        agent: AgentKind,
    ) -> Result<MaterializeReport, ComposeError> {
        self.sync_config.enable_sync();
        self.materialize_session(session_id, agent)
    }

    /// Generates a secret-scrubbed FUSE view of `<machine-root>/.sessions/<id>/config/<agent>`.
    ///
    /// - Any file matching `SecretDenylist::is_secret_path` is excluded from
    ///   `readable_files` and listed in `denied_secret_paths`.
    /// - Any `.json` or `.toml` file is parsed and scrubbed of all
    ///   `SecretDenylist` keys before being exposed.
    pub fn sanitized_view(
        &self,
        session_id: &str,
        agent: AgentKind,
    ) -> Result<SanitizedConfigView, ComposeError> {
        let session_dir = self.session_config_dir(session_id, agent);
        let mut view = SanitizedConfigView::default();
        if !session_dir.exists() {
            return Ok(view);
        }
        walk_sanitized(&session_dir, &session_dir, &self.denylist, &mut view)?;
        Ok(view)
    }

    fn compose_skills(
        &self,
        session_dir: &Path,
        local_agent_dir: &Path,
        synced_agent_dir: Option<&Path>,
        entries: &[&str],
        policy: EffectiveSyncPolicy,
    ) -> Result<(), ComposeError> {
        let sync_skills = policy.category_synced(SyncCategory::Skills);

        for entry_name in entries {
            let dst = session_dir.join(entry_name);
            let local_entry = local_agent_dir.join(entry_name);
            let synced_entry = synced_agent_dir.map(|d| d.join(entry_name));

            if sync_skills {
                if let Some(host_dir) = synced_entry.filter(|p| p.exists()) {
                    match policy.skills_mode {
                        SkillsMode::HostOnly => {
                            atomic_symlink(&host_dir, &dst)?;
                        }
                        SkillsMode::HostWins => {
                            // Materialize a real directory at `dst` containing symlinks
                            // to local entries first, then host entries (host wins collisions).
                            if fs::symlink_metadata(&dst)
                                .map(|m| m.file_type().is_symlink() || m.is_file())
                                .unwrap_or(false)
                            {
                                remove_path_any(&dst)?;
                            }
                            fs::create_dir_all(&dst).map_err(|source| ComposeError::Io {
                                path: dst.clone(),
                                source,
                            })?;

                            let mut expected_children = BTreeSet::new();
                            if local_entry.is_dir() {
                                for child in read_dir_sorted(&local_entry)? {
                                    let name = child.file_name();
                                    let child_dst = dst.join(&name);
                                    atomic_symlink(&child.path(), &child_dst)?;
                                    expected_children.insert(name);
                                }
                            }
                            if host_dir.is_dir() {
                                for child in read_dir_sorted(&host_dir)? {
                                    let name = child.file_name();
                                    let child_dst = dst.join(&name);
                                    atomic_symlink(&child.path(), &child_dst)?;
                                    expected_children.insert(name);
                                }
                            }
                            // Prune stale children no longer present in local or host.
                            for existing in read_dir_sorted(&dst)? {
                                if !expected_children.contains(&existing.file_name()) {
                                    remove_path_any(&existing.path())?;
                                }
                            }
                        }
                    }
                    continue;
                }
            }

            // Sync OFF (or host bundle lacks this entry): point directly to local_home.
            if local_entry.exists() {
                atomic_symlink(&local_entry, &dst)?;
            } else if fs::symlink_metadata(&dst).is_ok() {
                remove_path_any(&dst)?;
            }
        }

        Ok(())
    }

    fn compose_instructions(
        &self,
        session_dir: &Path,
        local_agent_dir: &Path,
        synced_agent_dir: Option<&Path>,
        files: &[&str],
        policy: EffectiveSyncPolicy,
    ) -> Result<(), ComposeError> {
        let sync_instructions = policy.category_synced(SyncCategory::Instructions);

        for filename in files {
            let dst = session_dir.join(filename);
            let local_file = local_agent_dir.join(filename);
            let synced_file = synced_agent_dir.map(|d| d.join(filename));

            if sync_instructions {
                if let Some(host_file) = synced_file.filter(|p| p.is_file()) {
                    let bytes = fs::read(&host_file).map_err(|source| ComposeError::Io {
                        path: host_file,
                        source,
                    })?;
                    atomic_write(&dst, &bytes)?;
                    continue;
                }
            }

            // Sync OFF (or rage-quit): point directly to local_home file.
            if local_file.exists() {
                atomic_symlink(&local_file, &dst)?;
            } else if fs::symlink_metadata(&dst).is_ok() {
                remove_path_any(&dst)?;
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn compose_mcp_and_settings(
        &self,
        session_dir: &Path,
        local_agent_dir: &Path,
        synced_agent_dir: Option<&Path>,
        agent: AgentKind,
        policy: EffectiveSyncPolicy,
        ctx: &RewriteContext,
        violations: &mut Vec<RewriteViolation>,
    ) -> Result<(), ComposeError> {
        let spec = agent.spec();
        let sync_mcp = policy.category_synced(SyncCategory::Mcp);
        let sync_settings = policy.category_synced(SyncCategory::Settings);

        // Special case: Codex stores both settings and `[mcp_servers]` in `config.toml`.
        let mcp_combined_with_settings = spec.settings_files.contains(&spec.mcp_config_file);

        if mcp_combined_with_settings {
            let filename = spec.mcp_config_file;
            let local_path = local_agent_dir.join(filename);
            let synced_path = synced_agent_dir.map(|d| d.join(filename));

            let local_doc = read_structured_opt(&local_path, spec.mcp_format)?;
            let synced_doc = match synced_path.filter(|p| p.is_file()) {
                Some(p) => read_structured_opt(&p, spec.mcp_format)?,
                None => None,
            };

            let mut merged = JsonMap::new();

            // Non-MCP settings keys.
            let settings_source = if sync_settings {
                synced_doc.as_ref().or(local_doc.as_ref())
            } else {
                local_doc.as_ref()
            };
            if let Some(obj) = settings_source.and_then(JsonValue::as_object) {
                for (k, v) in obj {
                    if k != spec.mcp_root_key {
                        merged.insert(k.clone(), v.clone());
                    }
                }
            }

            // MCP servers table.
            let mcp_source = if sync_mcp {
                synced_doc.as_ref().or(local_doc.as_ref())
            } else {
                local_doc.as_ref()
            };
            if let Some(servers) = mcp_source
                .and_then(JsonValue::as_object)
                .and_then(|o| o.get(spec.mcp_root_key))
            {
                merged.insert(spec.mcp_root_key.to_string(), servers.clone());
            }

            let mut doc = JsonValue::Object(merged);
            if sync_mcp || sync_settings {
                let report = self.rewrite_pipeline.run_safe(
                    &doc,
                    ctx,
                    spec.mcp_format,
                    Some(&self.protected_humata_mcp),
                );
                doc = report.output;
                violations.extend(report.violations);
            } else {
                // Local mode: still ensure protected `humata` MCP is present without mutating local_home.
                let _ = enforce_protected_humata_mcp(&mut doc, agent, &self.protected_humata_mcp);
            }

            write_structured(&session_dir.join(filename), &doc, spec.mcp_format)?;
            return Ok(());
        }

        // Separate MCP config file.
        {
            let mcp_file = spec.mcp_config_file;
            let local_mcp = local_agent_dir.join(mcp_file);
            let synced_mcp = synced_agent_dir.map(|d| d.join(mcp_file));

            let mut doc = if sync_mcp {
                match synced_mcp.filter(|p| p.is_file()) {
                    Some(p) => read_structured_opt(&p, spec.mcp_format)?
                        .unwrap_or_else(|| JsonValue::Object(JsonMap::new())),
                    None => read_structured_opt(&local_mcp, spec.mcp_format)?
                        .unwrap_or_else(|| JsonValue::Object(JsonMap::new())),
                }
            } else {
                read_structured_opt(&local_mcp, spec.mcp_format)?
                    .unwrap_or_else(|| JsonValue::Object(JsonMap::new()))
            };

            if sync_mcp {
                let report = self.rewrite_pipeline.run_safe(
                    &doc,
                    ctx,
                    spec.mcp_format,
                    Some(&self.protected_humata_mcp),
                );
                doc = report.output;
                violations.extend(report.violations);
            } else {
                let _ = enforce_protected_humata_mcp(&mut doc, agent, &self.protected_humata_mcp);
            }

            write_structured(&session_dir.join(mcp_file), &doc, spec.mcp_format)?;
        }

        // Separate Settings files.
        for settings_file in spec.settings_files {
            let dst = session_dir.join(settings_file);
            let local_file = local_agent_dir.join(settings_file);
            let synced_file = synced_agent_dir.map(|d| d.join(settings_file));
            let fmt = format_for_path(Path::new(settings_file));

            if sync_settings {
                if let Some(host_file) = synced_file.filter(|p| p.is_file()) {
                    if let Some(doc) = read_structured_opt(&host_file, fmt)? {
                        // Scrub any accidental secrets from synced settings, then apply rewrite pipeline.
                        let scrubbed = self.denylist.scrub_json(&doc);
                        let report = self.rewrite_pipeline.run_safe(&scrubbed, ctx, fmt, None);
                        violations.extend(report.violations);
                        write_structured(&dst, &report.output, fmt)?;
                        continue;
                    }
                }
            }

            // Sync OFF (or rage-quit): link directly to local_home settings if present.
            if local_file.exists() {
                atomic_symlink(&local_file, &dst)?;
            } else if fs::symlink_metadata(&dst).is_ok() {
                remove_path_any(&dst)?;
            }
        }

        Ok(())
    }
}

fn format_for_path(path: &Path) -> ConfigFormat {
    match path.extension().and_then(|e| e.to_str()) {
        Some("toml") => ConfigFormat::Toml,
        _ => ConfigFormat::Json,
    }
}

fn read_structured_opt(
    path: &Path,
    format: ConfigFormat,
) -> Result<Option<JsonValue>, ComposeError> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(path).map_err(|source| ComposeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    match format {
        ConfigFormat::Json => {
            let val: JsonValue =
                serde_json::from_str(&text).map_err(|source| ComposeError::InvalidJson {
                    path: path.to_path_buf(),
                    source,
                })?;
            Ok(Some(val))
        }
        ConfigFormat::Toml => {
            let val: toml::Value =
                toml::from_str(&text).map_err(|source| ComposeError::InvalidToml {
                    path: path.to_path_buf(),
                    source,
                })?;
            Ok(Some(toml_to_json(&val)))
        }
    }
}

fn write_structured(
    path: &Path,
    value: &JsonValue,
    format: ConfigFormat,
) -> Result<(), ComposeError> {
    let bytes = match format {
        ConfigFormat::Json => {
            let mut s = serde_json::to_string_pretty(value).expect("serializing serde_json::Value");
            s.push('\n');
            s.into_bytes()
        }
        ConfigFormat::Toml => {
            let toml_val = json_to_toml(value).map_err(|e| ComposeError::TomlSerialize {
                path: path.to_path_buf(),
                message: e.to_string(),
            })?;
            let s = toml::to_string_pretty(&toml_val).map_err(|e| ComposeError::TomlSerialize {
                path: path.to_path_buf(),
                message: e.to_string(),
            })?;
            s.into_bytes()
        }
    };
    atomic_write(path, &bytes)
}

/// Atomically writes `bytes` to `dst` using a sibling temp file + `fs::rename`.
pub fn atomic_write(dst: &Path, bytes: &[u8]) -> Result<(), ComposeError> {
    let parent = dst.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| ComposeError::Io {
        path: parent.to_path_buf(),
        source,
    })?;

    // If `dst` is currently a directory (not a symlink), remove it before renaming a file over it.
    if let Ok(meta) = fs::symlink_metadata(dst) {
        if meta.is_dir() && !meta.file_type().is_symlink() {
            fs::remove_dir_all(dst).map_err(|source| ComposeError::Io {
                path: dst.to_path_buf(),
                source,
            })?;
        }
    }

    let tmp_path = unique_tmp_path(parent, dst);
    let mut file = fs::File::create(&tmp_path).map_err(|source| ComposeError::Io {
        path: tmp_path.clone(),
        source,
    })?;
    file.write_all(bytes).map_err(|source| ComposeError::Io {
        path: tmp_path.clone(),
        source,
    })?;
    file.sync_all().map_err(|source| ComposeError::Io {
        path: tmp_path.clone(),
        source,
    })?;
    drop(file);

    fs::rename(&tmp_path, dst).map_err(|source| {
        let _ = fs::remove_file(&tmp_path);
        ComposeError::Io {
            path: dst.to_path_buf(),
            source,
        }
    })
}

/// Atomically replaces `dst` with a symlink pointing to `target`.
pub fn atomic_symlink(target: &Path, dst: &Path) -> Result<(), ComposeError> {
    let parent = dst.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| ComposeError::Io {
        path: parent.to_path_buf(),
        source,
    })?;

    // If `dst` is a real directory (not a symlink), remove it so `rename` can place a symlink there.
    if let Ok(meta) = fs::symlink_metadata(dst) {
        if meta.is_dir() && !meta.file_type().is_symlink() {
            fs::remove_dir_all(dst).map_err(|source| ComposeError::Io {
                path: dst.to_path_buf(),
                source,
            })?;
        }
    }

    let tmp_link = unique_tmp_path(parent, dst);
    create_symlink(target, &tmp_link).map_err(|source| ComposeError::Io {
        path: tmp_link.clone(),
        source,
    })?;

    fs::rename(&tmp_link, dst).map_err(|source| {
        let _ = fs::remove_file(&tmp_link);
        ComposeError::Io {
            path: dst.to_path_buf(),
            source,
        }
    })
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

fn unique_tmp_path(parent: &Path, dst: &Path) -> PathBuf {
    let name = dst.file_name().and_then(|n| n.to_str()).unwrap_or("config");
    let seq = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    parent.join(format!(".humata-tmp.{name}.{pid}.{seq}"))
}

fn remove_path_any(path: &Path) -> Result<(), ComposeError> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(ComposeError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if meta.is_dir() && !meta.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|source| ComposeError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn read_dir_sorted(dir: &Path) -> Result<Vec<fs::DirEntry>, ComposeError> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).map_err(|source| ComposeError::Io {
        path: dir.to_path_buf(),
        source,
    })? {
        entries.push(entry.map_err(|source| ComposeError::Io {
            path: dir.to_path_buf(),
            source,
        })?);
    }
    entries.sort_by_key(|e| e.file_name());
    Ok(entries)
}

fn walk_sanitized(
    root: &Path,
    dir: &Path,
    denylist: &SecretDenylist,
    view: &mut SanitizedConfigView,
) -> Result<(), ComposeError> {
    for entry in read_dir_sorted(dir)? {
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();

        // Check denylist BEFORE following symlinks or reading contents.
        if denylist.is_secret_path(&rel) {
            view.denied_secret_paths.insert(rel);
            continue;
        }

        // Follow symlinks via `fs::metadata` for non-secret entries.
        let meta = match fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue, // Skip broken symlinks safely.
        };

        if meta.is_dir() {
            walk_sanitized(root, &path, denylist, view)?;
        } else if meta.is_file() {
            let raw = fs::read(&path).map_err(|source| ComposeError::Io {
                path: path.clone(),
                source,
            })?;
            let sanitized = sanitize_file_bytes(&rel, &raw, denylist);
            view.readable_files.insert(rel, sanitized);
        }
    }
    Ok(())
}

fn sanitize_file_bytes(rel_path: &Path, raw: &[u8], denylist: &SecretDenylist) -> Vec<u8> {
    match rel_path.extension().and_then(|e| e.to_str()) {
        Some("json") => {
            if let Ok(parsed) = serde_json::from_slice::<JsonValue>(raw) {
                let scrubbed = denylist.scrub_json(&parsed);
                if let Ok(mut pretty) = serde_json::to_string_pretty(&scrubbed) {
                    pretty.push('\n');
                    return pretty.into_bytes();
                }
            }
            raw.to_vec()
        }
        Some("toml") => {
            if let Ok(text) = std::str::from_utf8(raw) {
                if let Ok(parsed) = toml::from_str::<toml::Value>(text) {
                    let scrubbed = denylist.scrub_toml(&parsed);
                    if let Ok(pretty) = toml::to_string_pretty(&scrubbed) {
                        return pretty.into_bytes();
                    }
                }
            }
            raw.to_vec()
        }
        _ => raw.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MachineSyncOverride;
    use crate::rewrite::{
        BashFilterConfig, DeclarativeRules, RewriteError, SchemeScriptConfig, run_bash_filter,
        run_scheme_transform,
    };
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;
    use tempfile::TempDir;

    fn write_test_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(path, content).expect("write test file");
    }

    fn test_protected_mcp() -> ProtectedHumataMcp {
        ProtectedHumataMcp::new(
            "/usr/local/bin/humata2",
            vec![
                "mcp".to_string(),
                "--socket".to_string(),
                "/tmp/humata.sock".to_string(),
            ],
        )
    }

    fn make_test_composer(root: &TempDir) -> ConfigComposer {
        let machine_root = root.path().join("machine-root");
        let local_home = root.path().join("fake-home");
        let synced_root = root.path().join("machine-root/.humata/synced-config/gen-1");
        fs::create_dir_all(&machine_root).unwrap();
        fs::create_dir_all(&local_home).unwrap();
        fs::create_dir_all(&synced_root).unwrap();

        let mut composer =
            ConfigComposer::new(machine_root, local_home, "gpu-box", test_protected_mcp());
        composer.synced_bundle_root = Some(synced_root);
        composer.os = "linux".to_string();
        composer.arch = "x86_64".to_string();
        composer
    }

    #[test]
    fn all_four_agents_materialize_auth_passthrough_and_sanitized_fuse_view() {
        let tmp = TempDir::new().unwrap();
        let composer = make_test_composer(&tmp);
        let home = &composer.local_home;
        let synced = composer.synced_bundle_root.as_ref().unwrap();

        // Seed machine-local auth + local configs + host synced configs for all 4 agents.
        // 1. Claude
        write_test_file(
            &home.join(".claude/.credentials.json"),
            r#"{"access_token":"secret-claude-tok","refresh_token":"secret-claude-ref"}"#,
        );
        write_test_file(
            &home.join(".claude/settings.json"),
            r#"{"theme":"dark","api_key":"sk-ant-secret"}"#,
        );
        write_test_file(
            &synced.join("claude/CLAUDE.md"),
            "# Host Claude Instructions\n",
        );
        write_test_file(
            &synced.join("claude/skills/deploy/SKILL.md"),
            "# Host Deploy Skill\n",
        );

        // 2. Codex
        write_test_file(
            &home.join(".codex/auth.json"),
            r#"{"OPENAI_API_KEY":"sk-openai-secret"}"#,
        );
        write_test_file(
            &home.join(".codex/config.toml"),
            "model = \"o3\"\napi_key = \"inline-codex-secret\"\n",
        );
        write_test_file(&synced.join("codex/AGENTS.md"), "# Host Codex Agents\n");
        write_test_file(
            &synced.join("codex/skills/review/SKILL.md"),
            "# Host Review\n",
        );

        // 3. Copilot
        write_test_file(
            &home.join(".config/gh/hosts.yml"),
            "github.com:\n  oauth_token: gho_secret123\n",
        );
        write_test_file(
            &home.join(".copilot/config.json"),
            r#"{"editor":"vim","github_token":"ghp_inline_secret"}"#,
        );
        write_test_file(
            &synced.join("copilot/copilot-instructions.md"),
            "# Host Copilot Instructions\n",
        );
        write_test_file(&synced.join("copilot/skills/triage/SKILL.md"), "# Triage\n");

        // 4. Agy
        write_test_file(
            &home.join(".gemini/oauth_creds.json"),
            r#"{"access_token":"ya29.secret","refresh_token":"1//secret"}"#,
        );
        write_test_file(
            &home.join(".gemini/antigravity-cli/settings.json"),
            r#"{"model":"gemini-2.5-pro","gemini_api_key":"AIzaSecret"}"#,
        );
        write_test_file(&synced.join("agy/GEMINI.md"), "# Host Agy Instructions\n");
        write_test_file(&synced.join("agy/skills/plan/SKILL.md"), "# Plan Skill\n");

        for agent in AgentKind::ALL {
            let report = composer
                .materialize_session("sess-001", agent)
                .unwrap_or_else(|e| panic!("materialize {agent} failed: {e}"));

            assert!(!report.holder_restart_required);
            assert!(report.session_config_dir.is_dir());
            assert_eq!(
                report.env_plan.env_vars.get(agent.config_env_var()),
                Some(&report.session_config_dir.to_string_lossy().into_owned())
            );
            assert!(
                !report.auth_passthrough_links.is_empty(),
                "expected local auth pass-through symlink for {agent}"
            );
            for link in &report.auth_passthrough_links {
                let meta = fs::symlink_metadata(link).unwrap();
                assert!(
                    meta.file_type().is_symlink(),
                    "auth must be a symlink for {agent}"
                );
            }

            // Verify sanitized FUSE view strips ALL secret files and inline secret keys.
            let view = composer.sanitized_view("sess-001", agent).unwrap();
            assert!(
                !view.denied_secret_paths.is_empty(),
                "expected secret auth files to be denied in FUSE view for {agent}"
            );
            for (rel, bytes) in &view.readable_files {
                let text = String::from_utf8_lossy(bytes);
                assert!(
                    !text.contains("secret")
                        && !text.contains("gho_")
                        && !text.contains("ghp_")
                        && !text.contains("AIza"),
                    "secret leaked into sanitized FUSE view for {agent} at {rel:?}: {text}"
                );
            }
        }

        // Verify exact denied auth paths per agent.
        assert!(
            composer
                .sanitized_view("sess-001", AgentKind::Claude)
                .unwrap()
                .is_denied(".credentials.json")
        );
        assert!(
            composer
                .sanitized_view("sess-001", AgentKind::Codex)
                .unwrap()
                .is_denied("auth.json")
        );
        assert!(
            composer
                .sanitized_view("sess-001", AgentKind::Copilot)
                .unwrap()
                .is_denied("hosts.yml")
        );
        assert!(
            composer
                .sanitized_view("sess-001", AgentKind::Agy)
                .unwrap()
                .is_denied("oauth_creds.json")
        );
    }

    #[test]
    fn sync_defaults_per_machine_overrides_and_instant_rage_quit() {
        // 1. Verify defaults: skills & instructions ON; mcp & settings OFF.
        let cfg = SyncConfig::default();
        assert!(cfg.sync_enabled);
        assert!(cfg.toggles.skills);
        assert!(cfg.toggles.instructions);
        assert!(!cfg.toggles.mcp);
        assert!(!cfg.toggles.settings);

        // 2. Parse TOML with per-machine overrides.
        let toml_cfg = SyncConfig::from_toml_str(
            r#"
[sync]
enabled = true
skills = true
instructions = true
mcp = false
settings = false

[sync.machines."gpu-box"]
mcp = true
settings = true

[sync.machines."airgapped-*"]
enabled = false
"#,
        )
        .unwrap();

        let gpu = toml_cfg.effective_for_machine("gpu-box");
        assert!(gpu.category_synced(SyncCategory::Skills));
        assert!(gpu.category_synced(SyncCategory::Instructions));
        assert!(gpu.category_synced(SyncCategory::Mcp));
        assert!(gpu.category_synced(SyncCategory::Settings));

        let air = toml_cfg.effective_for_machine("airgapped-01");
        assert!(!air.sync_enabled);
        assert!(!air.category_synced(SyncCategory::Skills));
        assert!(!air.category_synced(SyncCategory::Mcp));

        let other = toml_cfg.effective_for_machine("laptop");
        assert!(other.category_synced(SyncCategory::Skills));
        assert!(other.category_synced(SyncCategory::Instructions));
        assert!(!other.category_synced(SyncCategory::Mcp));
        assert!(!other.category_synced(SyncCategory::Settings));

        // 3. Test instant non-destructive rage-quit on real tempdirs.
        let tmp = TempDir::new().unwrap();
        let mut composer = make_test_composer(&tmp);
        composer.sync_config.machines.insert(
            "gpu-box".to_string(),
            MachineSyncOverride {
                mcp: Some(true),
                settings: Some(true),
                ..Default::default()
            },
        );

        let local_claude_md = composer.local_home.join(".claude/CLAUDE.md");
        let local_skill = composer
            .local_home
            .join(".claude/skills/local-only/SKILL.md");
        let local_creds = composer.local_home.join(".claude/.credentials.json");
        write_test_file(&local_claude_md, "# Machine-Local Instructions\n");
        write_test_file(&local_skill, "# Local Skill\n");
        write_test_file(&local_creds, r#"{"access_token":"keep-me-intact"}"#);

        let synced = composer.synced_bundle_root.as_ref().unwrap();
        write_test_file(
            &synced.join("claude/CLAUDE.md"),
            "# Host Synced Instructions\n",
        );
        write_test_file(
            &synced.join("claude/skills/host-skill/SKILL.md"),
            "# Host Skill\n",
        );

        // Initially synced: CLAUDE.md comes from host.
        let before = composer
            .materialize_session("sess-rq", AgentKind::Claude)
            .unwrap();
        assert!(before.effective_policy.sync_enabled);
        let session_md = before.session_config_dir.join("CLAUDE.md");
        assert_eq!(
            fs::read_to_string(&session_md).unwrap(),
            "# Host Synced Instructions\n"
        );
        assert!(
            before
                .session_config_dir
                .join("skills/host-skill/SKILL.md")
                .exists()
        );

        // Trigger instant rage-quit!
        let after = composer.disable_sync("sess-rq", AgentKind::Claude).unwrap();
        assert!(!after.effective_policy.sync_enabled);
        assert!(!after.holder_restart_required);

        // Session now points cleanly to machine-local CLAUDE.md & skills!
        assert!(
            fs::symlink_metadata(&session_md)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(&session_md).unwrap(),
            "# Machine-Local Instructions\n"
        );
        assert!(
            after
                .session_config_dir
                .join("skills/local-only/SKILL.md")
                .exists()
        );
        assert!(
            !after
                .session_config_dir
                .join("skills/host-skill/SKILL.md")
                .exists()
        );

        // Real local_home files are completely untouched.
        assert_eq!(
            fs::read_to_string(&local_claude_md).unwrap(),
            "# Machine-Local Instructions\n"
        );
        assert_eq!(
            fs::read_to_string(&local_creds).unwrap(),
            r#"{"access_token":"keep-me-intact"}"#
        );

        // Re-enabling sync restores host layer cleanly.
        let reenabled = composer.enable_sync("sess-rq", AgentKind::Claude).unwrap();
        assert!(reenabled.effective_policy.sync_enabled);
        assert_eq!(
            fs::read_to_string(&session_md).unwrap(),
            "# Host Synced Instructions\n"
        );
    }

    #[test]
    fn three_stage_rewrite_pipeline_and_step_budget_and_bash_recovery() {
        let tmp = TempDir::new().unwrap();
        let scripts_dir = tmp.path().join("scripts");
        fs::create_dir_all(&scripts_dir).unwrap();

        let mut ctx = RewriteContext {
            machine_id: "m-gpu-1".to_string(),
            machine_name: "gpu-box".to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            home_dir: PathBuf::from("/home/ubuntu"),
            agent: AgentKind::Claude,
            local_env: BTreeMap::new(),
        };
        ctx.local_env.insert(
            "LOCAL_GH_TOKEN".to_string(),
            "tok-from-target-env".to_string(),
        );

        // Stage 1: Declarative TOML rules.
        let rules = DeclarativeRules::from_toml_str(
            r#"
[[path_rewrite]]
machines = ["gpu-*"]
from_prefix = "/opt/homebrew/bin/"
to_prefix = "/usr/local/bin/"

[mcp.servers."apple-notes"]
machines = ["gpu-*"]
enabled = false

[mcp.servers."github"]
command = "npx"
pass_env = ["LOCAL_GH_TOKEN"]
"#,
        )
        .unwrap();

        // Stage 2: Scheme AST transform (drops macos-only server, rewrites /Users/ -> /home/ubuntu/).
        let scheme = SchemeScriptConfig::new(
            r#"
(define (transform ctx)
  (let ((os (hash-get ctx 'os))
        (cfg (hash-get ctx 'config)))
    (if (equal? os "linux")
        (drop-mcp-server
          (rewrite-prefix cfg "/Users/host/" "/home/ubuntu/")
          "macos-spotlight")
        cfg)))
"#,
        );

        // Stage 3: Target-local bash filter adding a marker field.
        let good_script = scripts_dir.join("tag-target.sh");
        write_test_file(
            &good_script,
            "#!/usr/bin/env bash\nset -euo pipefail\ncat | sed 's/}$/,\"rewrittenByBash\":\"'\"$HUMATA_MACHINE\"'\"}/'\n",
        );
        fs::set_permissions(&good_script, fs::Permissions::from_mode(0o755)).unwrap();

        let pipeline = RewritePipeline {
            declarative: rules,
            scheme: Some(scheme),
            bash: Some(BashFilterConfig::new(&scripts_dir, "tag-target.sh")),
        };

        let input = json!({
            "mcpServers": {
                "humata": test_protected_mcp().entry,
                "apple-notes": { "command": "/opt/homebrew/bin/notes-mcp" },
                "macos-spotlight": { "command": "/usr/bin/mdfind-mcp" },
                "github": { "command": "/opt/homebrew/bin/github-mcp", "args": ["/Users/host/work"] }
            }
        });

        let report = pipeline.run_safe(
            &input,
            &ctx,
            ConfigFormat::Json,
            Some(&test_protected_mcp()),
        );
        assert!(report.stage1_applied);
        assert!(report.stage2_applied);
        assert!(report.stage3_applied);
        assert!(
            report.violations.is_empty(),
            "unexpected violations: {:?}",
            report.violations
        );

        let servers = report.output["mcpServers"].as_object().unwrap();
        assert!(
            !servers.contains_key("apple-notes"),
            "Stage 1 should drop apple-notes"
        );
        assert!(
            !servers.contains_key("macos-spotlight"),
            "Stage 2 should drop macos-spotlight"
        );
        assert_eq!(servers["github"]["command"], "npx");
        assert_eq!(servers["github"]["args"][0], "/home/ubuntu/work");
        assert_eq!(
            servers["github"]["env"]["LOCAL_GH_TOKEN"],
            "tok-from-target-env"
        );
        assert_eq!(report.output["rewrittenByBash"], "gpu-box");

        // Test Scheme step-budget timeout on infinite recursion.
        let infinite_scheme = SchemeScriptConfig::new(
            r#"
(define (spin n) (spin (+ n 1)))
(define (transform ctx) (spin 0))
"#,
        )
        .with_step_budget(200)
        .with_max_depth(500);

        let err = run_scheme_transform(&infinite_scheme, &input, &ctx).unwrap_err();
        assert!(
            matches!(err, RewriteError::StepBudgetExceeded { .. }),
            "expected StepBudgetExceeded, got {err:?}"
        );

        // Test Scheme syntax error handling.
        let malformed_scheme = SchemeScriptConfig::new("(define (transform ctx) (unterminated");
        let syn_err = run_scheme_transform(&malformed_scheme, &input, &ctx).unwrap_err();
        assert!(matches!(syn_err, RewriteError::SchemeSyntax(_)));

        // Test bash filter malformed output recovery in `run_safe`.
        let bad_script = scripts_dir.join("broken.sh");
        write_test_file(
            &bad_script,
            "#!/usr/bin/env bash\necho 'NOT VALID JSON {{'\n",
        );
        fs::set_permissions(&bad_script, fs::Permissions::from_mode(0o755)).unwrap();

        let bad_bash = BashFilterConfig::new(&scripts_dir, "broken.sh");
        let bash_err = run_bash_filter(&bad_bash, &input, &ctx).unwrap_err();
        assert!(matches!(bash_err, RewriteError::BashInvalidOutput(_)));

        // Path traversal rejected.
        let traversal = BashFilterConfig::new(&scripts_dir, "../escape.sh");
        assert!(matches!(
            run_bash_filter(&traversal, &input, &ctx),
            Err(RewriteError::InvalidScriptName(_))
        ));

        // Bash timeout enforced cleanly.
        let slow_script = scripts_dir.join("slow.sh");
        write_test_file(&slow_script, "#!/usr/bin/env bash\nsleep 5\necho '{}'\n");
        fs::set_permissions(&slow_script, fs::Permissions::from_mode(0o755)).unwrap();
        let timeout_cfg =
            BashFilterConfig::new(&scripts_dir, "slow.sh").with_timeout(Duration::from_millis(80));
        assert!(matches!(
            run_bash_filter(&timeout_cfg, &input, &ctx),
            Err(RewriteError::BashTimeout(_))
        ));

        // Safe pipeline recovers from broken bash script and preserves valid config.
        let recovering_pipeline = RewritePipeline {
            declarative: DeclarativeRules::default(),
            scheme: None,
            bash: Some(bad_bash),
        };
        let recovered = recovering_pipeline.run_safe(
            &input,
            &ctx,
            ConfigFormat::Json,
            Some(&test_protected_mcp()),
        );
        assert!(!recovered.stage3_applied);
        assert!(recovered
            .violations
            .iter()
            .any(|v| matches!(v, RewriteViolation::StageFailedRecovered { stage, .. } if stage == "bash_filter")));
        assert_eq!(
            recovered.output["mcpServers"]["humata"],
            test_protected_mcp().entry
        );
    }

    #[test]
    fn post_transform_protection_blocks_humata_mcp_deletion_and_hijack() {
        let protected = test_protected_mcp();
        let ctx = RewriteContext {
            machine_id: "m-1".to_string(),
            machine_name: "worker-1".to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            home_dir: PathBuf::from("/home/worker"),
            agent: AgentKind::Claude,
            local_env: BTreeMap::new(),
        };

        let input = json!({
            "mcpServers": {
                "humata": protected.entry,
                "other": { "command": "other-mcp" }
            }
        });

        // Case 1: Stage 1 TOML rule tries to disable `humata`.
        let evil_toml = DeclarativeRules::from_toml_str(
            r#"
[mcp.servers."humata"]
enabled = false
"#,
        )
        .unwrap();
        let p1 = RewritePipeline {
            declarative: evil_toml,
            scheme: None,
            bash: None,
        };
        let r1 = p1.run_safe(&input, &ctx, ConfigFormat::Json, Some(&protected));
        assert_eq!(r1.output["mcpServers"]["humata"], protected.entry);
        assert!(
            r1.violations
                .iter()
                .any(|v| matches!(v, RewriteViolation::ProtectedHumataMcpTampered { .. }))
        );

        // Case 2: Stage 2 Scheme script tries to hijack `humata` command.
        let evil_scheme = SchemeScriptConfig::new(
            r#"
(define (transform ctx)
  (let ((cfg (hash-get ctx 'config)))
    (set-mcp-server cfg "humata" (hash "command" "/tmp/evil-mcp" "args" (list)))))
"#,
        );
        let p2 = RewritePipeline {
            declarative: DeclarativeRules::default(),
            scheme: Some(evil_scheme),
            bash: None,
        };
        let r2 = p2.run_safe(&input, &ctx, ConfigFormat::Json, Some(&protected));
        assert_eq!(r2.output["mcpServers"]["humata"], protected.entry);
        assert!(
            r2.violations
                .iter()
                .any(|v| matches!(v, RewriteViolation::ProtectedHumataMcpTampered { .. }))
        );

        // Case 3: Codex TOML format also restores `mcp_servers.humata` when deleted.
        let codex_ctx = RewriteContext {
            agent: AgentKind::Codex,
            ..ctx
        };
        let codex_input = json!({
            "model": "o3",
            "mcp_servers": {
                "humata": protected.entry
            }
        });
        let delete_scheme = SchemeScriptConfig::new(
            r#"(define (transform ctx) (drop-mcp-server (hash-get ctx 'config) "humata"))"#,
        );
        let p3 = RewritePipeline {
            declarative: DeclarativeRules::default(),
            scheme: Some(delete_scheme),
            bash: None,
        };
        let r3 = p3.run_safe(
            &codex_input,
            &codex_ctx,
            ConfigFormat::Toml,
            Some(&protected),
        );
        assert_eq!(r3.output["mcp_servers"]["humata"], protected.entry);
        assert!(
            r3.violations
                .iter()
                .any(|v| matches!(v, RewriteViolation::ProtectedHumataMcpTampered { .. }))
        );
    }
}
