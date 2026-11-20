//! `MountSpec` builder for macOS 26.7+ FSKit `/sbin/mount -F -o nobrowse`
//! invocations and `FSGenericURLResource` URLs.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Layout strategy for exposing an FSKit mountpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MountLayout {
    /// Mount directly at `mountpoint` with `-o nobrowse`.
    Direct { mountpoint: PathBuf },
    /// Mount at a hidden backing directory and expose `canonical_symlink`.
    BackingWithSymlink {
        backing_mountpoint: PathBuf,
        canonical_symlink: PathBuf,
    },
}

/// Specification for invoking `/sbin/mount -F -o nobrowse -t <fs_type> <url> <mountpoint>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountSpec {
    pub program: PathBuf,
    pub fs_type: String,
    pub resource_url: String,
    pub mountpoint: PathBuf,
    pub nobrowse: bool,
    pub extra_options: Vec<String>,
}

impl MountSpec {
    /// Builds a generic `FSGenericURLResource` URL (`<scheme>://<authority>[/<path>]`).
    ///
    /// Rejects empty or invalid RFC 3986 schemes and rejects `file` URLs.
    pub fn build_resource_url(
        scheme: &str,
        authority: &str,
        subpath: Option<&str>,
    ) -> io::Result<String> {
        validate_scheme(scheme)?;
        if authority.is_empty()
            || authority.contains('/')
            || authority.contains(' ')
            || authority.as_bytes().contains(&0)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid resource URL authority '{authority}'"),
            ));
        }
        match subpath {
            Some(p) if !p.is_empty() => {
                let trimmed = p.trim_start_matches('/');
                Ok(format!("{scheme}://{authority}/{trimmed}"))
            }
            _ => Ok(format!("{scheme}://{authority}")),
        }
    }

    /// Creates a new [`MountSpec`] with `nobrowse = true` (`MNT_DONTBROWSE`).
    pub fn new(
        fs_type: impl Into<String>,
        resource_url: impl Into<String>,
        mountpoint: impl Into<PathBuf>,
    ) -> io::Result<Self> {
        let fs_type = fs_type.into();
        let resource_url = resource_url.into();
        let mountpoint = mountpoint.into();

        if fs_type.is_empty() || fs_type.contains('/') || fs_type.contains(' ') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid filesystem type '{fs_type}'"),
            ));
        }
        let Some((scheme, rest)) = resource_url.split_once("://") else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("resource URL must include '://', got '{resource_url}'"),
            ));
        };
        validate_scheme(scheme)?;
        if rest.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("resource URL missing authority: '{resource_url}'"),
            ));
        }
        if mountpoint.as_os_str().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mountpoint cannot be empty",
            ));
        }

        Ok(Self {
            program: PathBuf::from("/sbin/mount"),
            fs_type,
            resource_url,
            mountpoint,
            nobrowse: true,
            extra_options: Vec::new(),
        })
    }

    /// Convenience constructor for `humatafs://...` mounts.
    pub fn for_humatafs(authority: &str, mountpoint: impl AsRef<Path>) -> io::Result<Self> {
        let url = Self::build_resource_url("humatafs", authority, None)?;
        Self::new("humatafs", url, mountpoint.as_ref())
    }

    /// Sets whether `-o nobrowse` is included (defaults to `true`).
    pub fn with_nobrowse(mut self, nobrowse: bool) -> Self {
        self.nobrowse = nobrowse;
        self
    }

    /// Appends an additional `-o` mount option (e.g., `"rdonly"`).
    pub fn with_option(mut self, opt: impl Into<String>) -> Self {
        self.extra_options.push(opt.into());
        self
    }

    /// Renders the argument vector passed to `/sbin/mount`.
    pub fn args(&self) -> Vec<String> {
        let mut args = vec!["-F".to_string()];

        let mut opts = Vec::new();
        if self.nobrowse {
            opts.push("nobrowse".to_string());
        }
        opts.extend(self.extra_options.iter().cloned());
        if !opts.is_empty() {
            args.push("-o".to_string());
            args.push(opts.join(","));
        }

        args.push("-t".to_string());
        args.push(self.fs_type.clone());
        args.push(self.resource_url.clone());
        args.push(self.mountpoint.to_string_lossy().into_owned());
        args
    }
}

fn validate_scheme(scheme: &str) -> io::Result<()> {
    let mut chars = scheme.chars();
    let Some(first) = chars.next() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "URL scheme cannot be empty",
        ));
    };
    if !first.is_ascii_alphabetic()
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid URL scheme '{scheme}'"),
        ));
    }
    if scheme.eq_ignore_ascii_case("file") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "file:// is not a valid FSGenericURLResource scheme",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_fskit_mount_args_with_nobrowse_for_humatafs_and_generic_schemes() {
        let spec = MountSpec::for_humatafs("main", "/Users/alice/.humata/fuse/main").unwrap();
        assert_eq!(spec.program, PathBuf::from("/sbin/mount"));
        assert_eq!(spec.resource_url, "humatafs://main");
        assert_eq!(
            spec.args(),
            vec![
                "-F",
                "-o",
                "nobrowse",
                "-t",
                "humatafs",
                "humatafs://main",
                "/Users/alice/.humata/fuse/main",
            ]
        );

        let generic_url =
            MountSpec::build_resource_url("myvfs+fuse", "workspace-1", Some("root")).unwrap();
        assert_eq!(generic_url, "myvfs+fuse://workspace-1/root");
        let generic_spec = MountSpec::new("myvfs", generic_url, "/tmp/mnt")
            .unwrap()
            .with_option("rdonly");
        assert_eq!(
            generic_spec.args(),
            vec![
                "-F",
                "-o",
                "nobrowse,rdonly",
                "-t",
                "myvfs",
                "myvfs+fuse://workspace-1/root",
                "/tmp/mnt",
            ]
        );
    }

    #[test]
    fn rejects_file_scheme_and_malformed_urls() {
        assert!(MountSpec::new("humatafs", "file:///tmp/x", "/tmp/mnt").is_err());
        assert!(MountSpec::new("humatafs", "not-a-url", "/tmp/mnt").is_err());
        assert!(MountSpec::build_resource_url("123bad", "main", None).is_err());
    }
}
