use crate::model::{Channel, ChannelId, MachineId};

pub(super) fn channels() -> Vec<Channel> {
    let channel = |id: &str, repos: &[&str], machines: &[&str], purpose: &str| Channel {
        id: ChannelId::new(id),
        name: id.to_owned(),
        repos: repos.iter().map(|repo| (*repo).to_owned()).collect(),
        machines: machines.iter().map(|id| MachineId::new(*id)).collect(),
        purpose: purpose.to_owned(),
        default_agent: None,
    };
    vec![
        channel(
            "web",
            &["acme/web", "acme/design-tokens", "acme/api"],
            &["this-mac", "gpu-box"],
            "The customer web app",
        ),
        channel(
            "api",
            &["acme/api", "acme/proto"],
            &["this-mac", "gpu-box"],
            "Public API and its clients",
        ),
        channel("infra", &["acme/infra"], &["ci-runner"], "Terraform and CI"),
        channel("scratch", &[], &["this-mac"], "No repos: notes and shells"),
    ]
}
