/// A host the bootstrap tools support.
#[derive(Clone, Copy)]
pub(crate) enum Platform {
    DarwinArm64,
    DarwinX86_64,
    LinuxArm64,
    LinuxX86_64,
}

impl Platform {
    pub(crate) const ALL: [Platform; 4] = [
        Platform::DarwinArm64,
        Platform::DarwinX86_64,
        Platform::LinuxArm64,
        Platform::LinuxX86_64,
    ];

    /// The name in the second column of `versions.lock`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Platform::DarwinArm64 => "darwin-arm64",
            Platform::DarwinX86_64 => "darwin-x86_64",
            Platform::LinuxArm64 => "linux-arm64",
            Platform::LinuxX86_64 => "linux-x86_64",
        }
    }
}
