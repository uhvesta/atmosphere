//! Settings › Plugins, and turning them on or off for a project; and where
//! a session's on-device model runs.

use humata_wire::{Reply, pb};

use super::Daemon;
use crate::convert;
use crate::error::DaemonError;

/// How long the list waits for each host before leaving it out.
const PLUGIN_LIST_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

impl Daemon {
    /// The plugins installed on the Mac and on each host that answers in
    /// time.
    pub(super) async fn list_plugins(&self) -> Result<pb::PluginList, DaemonError> {
        let mut asking = Vec::new();
        for host in self.remote_names() {
            let remote = self.remote(&host)?;
            let (hosts_file, build) = (self.hosts_file.clone(), self.build.clone());
            let answer = tokio::spawn(async move {
                let client = remote.client(&hosts_file, &build).await.ok()?;
                let request = pb::request::Kind::ScanPlugins(pb::ScanPlugins {});
                match remote.call(&client, request).await.ok()? {
                    pb::response::Kind::PluginsScanned(scanned) => Some(scanned.plugins),
                    _ => None,
                }
            });
            asking.push((host, answer));
        }
        let deadline = tokio::time::Instant::now() + PLUGIN_LIST_WAIT;
        let mut plugins = match self
            .forward("", pb::request::Kind::ScanPlugins(pb::ScanPlugins {}))
            .await?
        {
            Reply::Response(pb::response::Kind::PluginsScanned(scanned)) => scanned.plugins,
            _ => {
                return Err(DaemonError::Host {
                    host: String::new(),
                    source: humata_wire::ClientError::Unexpected {
                        expected: "the plugins found",
                        got: "another answer".to_owned(),
                    },
                });
            }
        };
        let mut unreachable = Vec::new();
        for (host, answer) in asking {
            match tokio::time::timeout_at(deadline, answer).await {
                Ok(Ok(Some(theirs))) => plugins.extend(theirs.into_iter().map(|mut plugin| {
                    plugin.host.clone_from(&host);
                    plugin
                })),
                _ => unreachable.push(host),
            }
        }
        Ok(pb::PluginList {
            plugins,
            unreachable,
        })
    }

    pub(super) fn set_project_plugin(
        &self,
        set: &pb::SetProjectPlugin,
    ) -> Result<pb::Project, DaemonError> {
        if set.id.is_empty() || set.agent() == pb::PluginAgent::Unspecified {
            return Err(DaemonError::BadPluginChoice);
        }
        self.store
            .set_project_plugin(&set.project_id, set.agent, &set.id, set.enabled)?;
        Ok(convert::project(self.store.project(&set.project_id)?))
    }

    /// Refuses a session on a host whose local-model server is the host's
    /// own, for one of Humata's on-device models: those run only on the
    /// Mac, which the host reaches through the forward to the same port.
    /// Every project's session defaults, as Get Session Defaults answers.
    #[inline(never)]
    pub(super) fn get_session_defaults(&self) -> Result<pb::response::Kind, DaemonError> {
        Ok(pb::response::Kind::SessionDefaults(
            self.store.session_defaults("")?.unwrap_or_default(),
        ))
    }

    /// Sets a project's session defaults, or every project's, and answers
    /// with the project or the defaults.
    #[inline(never)]
    pub(super) fn set_session_defaults(
        &self,
        set: &pb::SetSessionDefaults,
    ) -> Result<pb::response::Kind, DaemonError> {
        let defaults = set.defaults.as_ref();
        self.store.set_session_defaults(&set.project_id, defaults)?;
        if set.project_id.is_empty() {
            return Ok(pb::response::Kind::SessionDefaults(
                defaults.cloned().unwrap_or_default(),
            ));
        }
        let project = convert::project(self.store.project(&set.project_id)?);
        Ok(pb::response::Kind::ProjectUpdated(pb::ProjectUpdated {
            project: Some(project),
        }))
    }

    pub(super) async fn refuse_on_device_on_host(
        &self,
        start: &pb::StartSession,
    ) -> Result<(), DaemonError> {
        if start.host.is_empty() || start.backend() != pb::Backend::Local || start.model.is_empty()
        {
            return Ok(());
        }
        let Some(port) =
            serde_json::from_str::<humata_agent::LocalModelConfig>(&start.backend_config)
                .ok()
                .and_then(|config| config.base_url)
                .and_then(|url| loopback_port(&url))
        else {
            return Ok(());
        };
        if self.local.model_server().port() == Some(port) {
            return Ok(());
        }
        let list = pb::request::Kind::ListModels(pb::ListModels {
            backend: pb::Backend::Local.into(),
            ..Default::default()
        });
        // With the Mac's server down, the host's own can't be told from it.
        let Ok(Reply::Response(pb::response::Kind::Models(models))) = self.forward("", list).await
        else {
            return Ok(());
        };
        if models.models.iter().any(|model| model.id == start.model) {
            return Err(DaemonError::OnDeviceOnHost {
                host: start.host.clone(),
                model: start.model.clone(),
            });
        }
        Ok(())
    }
}

/// The port of `url` if it points at the machine it's read on.
fn loopback_port(url: &str) -> Option<u16> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split('/').next()?;
    let authority = authority.rsplit('@').next()?;
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !port.contains(']') => (host, port.parse().ok()?),
        _ => (authority, if scheme == "https" { 443 } else { 80 }),
    };
    matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "0.0.0.0").then_some(port)
}

#[cfg(test)]
mod tests {
    use super::loopback_port;

    #[test]
    fn only_urls_at_the_machine_itself_have_a_loopback_port() {
        assert_eq!(loopback_port("http://127.0.0.1:8123/v1"), Some(8123));
        assert_eq!(loopback_port("http://localhost/v1"), Some(80));
        assert_eq!(loopback_port("http://[::1]:11434"), Some(11434));
        assert_eq!(loopback_port("https://models.example.com:8123/v1"), None);
        assert_eq!(loopback_port("not a url"), None);
    }
}
