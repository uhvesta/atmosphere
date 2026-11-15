//! A machine's os and arch, as `uname -sm` reports them.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Os {
    Linux,
    Darwin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arch {
    X86_64,
    Aarch64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Platform {
    pub os: Os,
    pub arch: Arch,
}

impl Os {
    /// `Linux` or `linux`, `Darwin` or `darwin` (and `macos`).
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "linux" => Some(Self::Linux),
            "darwin" | "macos" => Some(Self::Darwin),
            _ => None,
        }
    }
}

impl Arch {
    /// Both spellings of each: `x86_64` and `amd64`, `aarch64` and `arm64`.
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "x86_64" | "amd64" => Some(Self::X86_64),
            "aarch64" | "arm64" => Some(Self::Aarch64),
            _ => None,
        }
    }
}

impl Platform {
    /// The last line of `output` that reads as `uname -sm`, so a login
    /// banner printed before it doesn't matter.
    pub(crate) fn from_uname(output: &str) -> Option<Self> {
        output.lines().rev().find_map(|line| {
            let mut words = line.split_whitespace();
            let os = Os::parse(words.next()?)?;
            let arch = Arch::parse(words.next()?)?;
            words.next().is_none().then_some(Self { os, arch })
        })
    }

    /// The last line of `output` shaped like `uname -sm` (two words, the
    /// first an OS name) that isn't a platform Humata knows, such as
    /// `Linux riscv64` or `FreeBSD amd64`: an answer to say is unsupported
    /// rather than unreadable.
    pub(crate) fn unknown_uname(output: &str) -> Option<String> {
        let line = output.lines().rev().find(|line| !line.trim().is_empty())?;
        let words: Vec<&str> = line.split_whitespace().collect();
        let shaped = words.len() == 2
            && words.iter().all(|word| {
                word.len() <= 32 && word.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            })
            && words[0].bytes().next().is_some_and(|b| b.is_ascii_alphabetic());
        (shaped && Self::from_uname(line).is_none()).then(|| words.join(" "))
    }
}

impl Platform {
    /// Every platform Humata ships, in the order bundles list them.
    pub const SHIPPED: [Self; 4] = [
        Self { os: Os::Darwin, arch: Arch::Aarch64 },
        Self { os: Os::Darwin, arch: Arch::X86_64 },
        Self { os: Os::Linux, arch: Arch::Aarch64 },
        Self { os: Os::Linux, arch: Arch::X86_64 },
    ];

    /// The platform this build of Humata runs on; none on one it doesn't
    /// ship.
    pub fn host() -> Option<Self> {
        let os = if cfg!(target_os = "linux") {
            Os::Linux
        } else if cfg!(target_os = "macos") {
            Os::Darwin
        } else {
            return None;
        };
        let arch = if cfg!(target_arch = "x86_64") {
            Arch::X86_64
        } else if cfg!(target_arch = "aarch64") {
            Arch::Aarch64
        } else {
            return None;
        };
        Some(Self { os, arch })
    }

    /// The folder name a tools bundle keeps this platform's files under:
    /// `darwin-arm64`, `darwin-x86_64`, `linux-aarch64`, `linux-x86_64`.
    pub fn bundle_name(self) -> &'static str {
        match (self.os, self.arch) {
            (Os::Darwin, Arch::Aarch64) => "darwin-arm64",
            (Os::Darwin, Arch::X86_64) => "darwin-x86_64",
            (Os::Linux, Arch::Aarch64) => "linux-aarch64",
            (Os::Linux, Arch::X86_64) => "linux-x86_64",
        }
    }

    pub(crate) fn from_bundle_name(name: &str) -> Option<Self> {
        Self::SHIPPED.into_iter().find(|platform| platform.bundle_name() == name)
    }
}

impl fmt::Display for Os {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Linux => "linux",
            Self::Darwin => "darwin",
        })
    }
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        })
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.os, self.arch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uname_reads_after_a_banner_in_either_spelling() {
        assert_eq!(
            Platform::from_uname("Welcome to corp!\nLinux aarch64\n"),
            Some(Platform {
                os: Os::Linux,
                arch: Arch::Aarch64
            })
        );
        assert_eq!(
            Platform::from_uname("Darwin arm64"),
            Some(Platform {
                os: Os::Darwin,
                arch: Arch::Aarch64
            })
        );
        assert_eq!(Platform::from_uname("Linux riscv64\n"), None);
        assert_eq!(Platform::from_uname("Linux x86_64 extra\n"), None);
    }

    #[test]
    fn an_unknown_platform_is_told_from_an_unreadable_answer() {
        assert_eq!(
            Platform::unknown_uname("Welcome\nLinux riscv64\n").as_deref(),
            Some("Linux riscv64")
        );
        assert_eq!(Platform::unknown_uname("Linux x86_64\n"), None);
        assert_eq!(Platform::unknown_uname("Last login: Mon from 10.0.0.1\n"), None);
        assert_eq!(Platform::unknown_uname(""), None);
    }
}
