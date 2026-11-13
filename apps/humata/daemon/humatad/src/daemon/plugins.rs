//! Settings › Plugins, and turning them on or off for a project.

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
}
