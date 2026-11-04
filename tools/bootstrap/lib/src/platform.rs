use crate::Error;

/// A host the bootstrap tools support.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Platform {
    DarwinArm64,
    DarwinX86_64,
    LinuxArm64,
    LinuxX86_64,
}

impl Platform {
    pub const ALL: [Platform; 4] = [
        Platform::DarwinArm64,
        Platform::DarwinX86_64,
        Platform::LinuxArm64,
        Platform::LinuxX86_64,
    ];

    /// The machine this runs on.
    pub fn host() -> Result<Self, Error> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => Ok(Platform::DarwinArm64),
            ("macos", "x86_64") => Ok(Platform::DarwinX86_64),
            ("linux", "aarch64") => Ok(Platform::LinuxArm64),
            ("linux", "x86_64") => Ok(Platform::LinuxX86_64),
            (os, arch) => Err(Error::UnsupportedPlatform { os, arch }),
        }
    }

    /// The name in the second column of `versions.lock`.
    pub fn name(self) -> &'static str {
        match self {
            Platform::DarwinArm64 => "darwin-arm64",
            Platform::DarwinX86_64 => "darwin-x86_64",
            Platform::LinuxArm64 => "linux-arm64",
            Platform::LinuxX86_64 => "linux-x86_64",
        }
    }
}
