use super::FakeDaemon;
use crate::daemon::{DaemonError, Event, Operation, Reply};
use crate::model::{AccessRule, Capability, ChannelId, GrantId, RuleSetting};

fn announce(inner: &mut super::Inner, channel: &ChannelId) {
    let rules = inner
        .workspace
        .rules
        .get(channel)
        .cloned()
        .unwrap_or_default();
    inner.emit(Event::Rules {
        channel: channel.clone(),
        rules,
    });
}

pub(super) fn set_rule(
    daemon: &FakeDaemon,
    channel: &ChannelId,
    capability: &Capability,
    setting: RuleSetting,
) -> Reply<()> {
    let channel = channel.clone();
    let capability = capability.clone();
    daemon.reply(
        Some(Operation::SetRule),
        format!("set_rule {channel} {capability} {setting:?}"),
        move |inner| {
            let rules = inner
                .workspace
                .rules
                .get_mut(&channel)
                .ok_or_else(|| DaemonError::NotFound {
                    what: format!("The access rules of channel {channel}"),
                })?;
            let rule: &mut AccessRule = rules
                .iter_mut()
                .find(|rule| rule.capability == capability)
                .ok_or_else(|| DaemonError::NotFound {
                    what: format!("The access rule {capability}"),
                })?;
            if rule.setting.is_none() {
                return Err(DaemonError::Refused {
                    what: format!("Changing “{}”", rule.label),
                    reason: "agents always have this access and it has no setting; leave it as it is"
                        .to_owned(),
                });
            }
            rule.setting = Some(setting);
            announce(inner, &channel);
            Ok(())
        },
    )
}

pub(super) fn remove_grant(daemon: &FakeDaemon, grant: &GrantId) -> Reply<()> {
    let grant = grant.clone();
    daemon.reply(
        Some(Operation::RemoveGrant),
        format!("remove_grant {grant}"),
        move |inner| {
            let found = inner.workspace.rules.iter_mut().find_map(|(channel, rules)| {
                rules.iter_mut().find_map(|rule| {
                    let at = rule.grants.iter().position(|each| each.id == grant)?;
                    rule.grants.remove(at);
                    Some(channel.clone())
                })
            });
            let channel = found.ok_or_else(|| DaemonError::NotFound {
                what: format!("The grant {grant}"),
            })?;
            announce(inner, &channel);
            Ok(())
        },
    )
}
