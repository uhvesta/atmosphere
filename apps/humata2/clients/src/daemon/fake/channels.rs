use super::FakeDaemon;
use crate::daemon::{DaemonError, Event, Operation, Reply};
use crate::model::{Channel, ChannelId, MachineId};

pub(super) fn create_channel(
    daemon: &FakeDaemon,
    name: &str,
    repos: Vec<String>,
    machines: Vec<MachineId>,
) -> Reply<ChannelId> {
    let name = name.trim().to_lowercase().replace(' ', "-");
    daemon.reply(
        Some(Operation::CreateChannel),
        format!("create channel {name:?}"),
        move |inner| {
            if name.is_empty() {
                return Err(DaemonError::Refused {
                    what: "Creating a channel".to_owned(),
                    reason: "it needs a name. Type one and try again".to_owned(),
                });
            }
            if inner
                .workspace
                .channels
                .iter()
                .any(|found| found.name == name)
            {
                return Err(DaemonError::Refused {
                    what: format!("Creating #{name}"),
                    reason: "a channel with that name exists. Pick another name".to_owned(),
                });
            }
            let id = ChannelId::new(name.clone());
            inner.workspace.channels.push(Channel {
                id: id.clone(),
                name,
                repos,
                machines,
                purpose: String::new(),
            });
            inner.emit(Event::Resync);
            Ok(id)
        },
    )
}
