use crate::model::{FileEntry, MachineId};

pub(super) fn files() -> Vec<FileEntry> {
    let file = |machine: &str, path: &str, size: &str, folder: bool| FileEntry {
        machine: MachineId::new(machine),
        path: path.to_owned(),
        size: size.to_owned(),
        folder,
    };
    vec![
        file("this-mac", "~/humata/web", "—", true),
        file("this-mac", "~/humata/web/README.md", "4 KB", false),
        file("this-mac", "~/humata/web/package.json", "2 KB", false),
        file("gpu-box", "~/humata/api", "—", true),
        file("gpu-box", "~/humata/api/Cargo.toml", "3 KB", false),
        file("gpu-box", "~/humata/web/build/app.tar.zst", "48 MB", false),
        file("ci-runner", "~/humata/infra/main.tf", "9 KB", false),
    ]
}
