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
}
