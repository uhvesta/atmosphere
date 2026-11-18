use crate::model::{Machine, MachineId, MachineKind};

pub(super) fn machines() -> Vec<Machine> {
    let machines = vec![
        Machine {
            id: MachineId::new("this-mac"),
            name: "This Mac".to_owned(),
            kind: MachineKind::ThisMac,
            online: true,
            detail: "macOS 26.7 · M4 Max · 64 GB".to_owned(),
            last_seen: "now".to_owned(),
            lost_at: None,
            stop: None,
            root: crate::desk::DEFAULT_ROOT.to_owned(),
        },
        Machine {
            id: MachineId::new("gpu-box"),
            name: "gpu-box".to_owned(),
            kind: MachineKind::Ssh {
                address: "dev@gpu-box.lan".to_owned(),
            },
            online: true,
            detail: "Ubuntu 26.04 · RTX 6090 · 128 GB".to_owned(),
            last_seen: "now".to_owned(),
            lost_at: None,
            stop: None,
            root: crate::desk::DEFAULT_ROOT.to_owned(),
        },
        Machine {
            id: MachineId::new("ci-runner"),
            name: "ci-runner".to_owned(),
            kind: MachineKind::Provider {
                provider: "Hetzner".to_owned(),
            },
            online: false,
            detail: "Debian 13 · 16 vCPU".to_owned(),
            last_seen: "14 min ago".to_owned(),
            lost_at: None,
            stop: None,
            root: crate::desk::DEFAULT_ROOT.to_owned(),
        },
    ];
    // Errors about these machines name them as the list does.
    for machine in &machines {
        crate::machine_names::remember(&machine.id, &machine.name);
    }
    machines
}
