use std::path::Path;
use std::process::Command;

use crate::LaunchdError;

/// The few launchd operations the installer needs.
pub trait Launchd: Send + Sync {
    fn bootstrap(&self, plist: &Path) -> Result<(), LaunchdError>;
    /// Unloading an agent that is not loaded is not an error.
    fn bootout(&self, label: &str) -> Result<(), LaunchdError>;
    fn is_loaded(&self, label: &str) -> Result<bool, LaunchdError>;
    fn kickstart(&self, label: &str) -> Result<(), LaunchdError>;
}

/// Talks to the real `/bin/launchctl` in this user's `gui/<uid>` domain.
#[derive(Debug, Default)]
pub struct SystemLaunchd;

struct Output {
    args: String,
    status: i32,
    stderr: String,
}

fn domain() -> String {
    format!("gui/{}", rustix::process::getuid().as_raw())
}

fn run(args: &[&str]) -> Result<Output, LaunchdError> {
    let joined = args.join(" ");
    let out = Command::new("/bin/launchctl")
        .args(args)
        .output()
        .map_err(|source| LaunchdError::Spawn { args: joined.clone(), source })?;
    Ok(Output {
        args: joined,
        status: out.status.code().unwrap_or(-1),
        stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
    })
}

fn ok(out: Output) -> Result<(), LaunchdError> {
    if out.status == 0 {
        Ok(())
    } else {
        Err(LaunchdError::Failed { args: out.args, status: out.status, stderr: out.stderr })
    }
}

impl Launchd for SystemLaunchd {
    fn bootstrap(&self, plist: &Path) -> Result<(), LaunchdError> {
        ok(run(&["bootstrap", &domain(), &plist.to_string_lossy()])?)
    }

    fn bootout(&self, label: &str) -> Result<(), LaunchdError> {
        let out = run(&["bootout", &format!("{}/{label}", domain())])?;
        // 3 = no such process (not loaded), 113 = could not find service.
        if matches!(out.status, 3 | 113) { Ok(()) } else { ok(out) }
    }

    fn is_loaded(&self, label: &str) -> Result<bool, LaunchdError> {
        Ok(run(&["print", &format!("{}/{label}", domain())])?.status == 0)
    }

    fn kickstart(&self, label: &str) -> Result<(), LaunchdError> {
        ok(run(&["kickstart", "-k", &format!("{}/{label}", domain())])?)
    }
}
