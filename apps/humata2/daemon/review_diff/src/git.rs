//! Running the git CLI in one repository, with an environment that can't
//! be steered by the caller's: no inherited `GIT_DIR` or `GIT_INDEX_FILE`,
//! no colours, no pager, no prompts, and no opportunistic index writes
//! (`GIT_OPTIONAL_LOCKS=0`), so reading a diff never touches the user's
//! index (D66).

use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

use crate::error::DiffError;

/// Variables a caller's environment may carry that would point git at
/// another repository, index or object store.
const SCRUBBED: [&str; 10] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_COMMON_DIR",
    "GIT_CEILING_DIRECTORIES",
    "GIT_EXTERNAL_DIFF",
    "GIT_DIFF_OPTS",
];

/// git in one working tree.
#[derive(Clone, Debug)]
pub(crate) struct Git {
    program: PathBuf,
    workdir: PathBuf,
    /// A private index to use instead of the user's (`GIT_INDEX_FILE`).
    index: Option<PathBuf>,
}

impl Git {
    pub(crate) fn new(program: PathBuf, workdir: PathBuf) -> Self {
        Self {
            program,
            workdir,
            index: None,
        }
    }

    pub(crate) fn program(&self) -> &Path {
        &self.program
    }

    pub(crate) fn workdir(&self) -> &Path {
        &self.workdir
    }

    /// The same repository, reading and writing `index` instead of the
    /// user's index.
    pub(crate) fn with_index(&self, index: &Path) -> Self {
        Self {
            index: Some(index.to_owned()),
            ..self.clone()
        }
    }

    pub(crate) fn command<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = Command::new(&self.program);
        command.current_dir(&self.workdir);
        for name in SCRUBBED {
            command.env_remove(name);
        }
        command
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_PAGER", "cat")
            .env("LC_ALL", "C")
            .env("GIT_AUTHOR_NAME", "Humata")
            .env("GIT_AUTHOR_EMAIL", "humata@localhost")
            .env("GIT_COMMITTER_NAME", "Humata")
            .env("GIT_COMMITTER_EMAIL", "humata@localhost")
            .args([
                "-c",
                "core.quotePath=false",
                "-c",
                "color.ui=never",
                "-c",
                "diff.noprefix=false",
                "-c",
                "diff.mnemonicPrefix=false",
                "-c",
                "diff.external=",
            ])
            .args(args)
            .stdin(Stdio::null());
        if let Some(index) = &self.index {
            command.env("GIT_INDEX_FILE", index);
        }
        command
    }

    /// Runs git and returns its stdout; a non-zero exit is an error.
    pub(crate) fn run<I, S>(&self, args: I) -> Result<Vec<u8>, DiffError>
    where
        I: IntoIterator<Item = S> + Clone,
        S: AsRef<OsStr>,
    {
        let output = self.output(args.clone(), None)?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(self.failed(args, &output))
        }
    }

    /// As [`Self::run`], feeding `input` on stdin.
    pub(crate) fn run_with_input<I, S>(&self, args: I, input: &[u8]) -> Result<Vec<u8>, DiffError>
    where
        I: IntoIterator<Item = S> + Clone,
        S: AsRef<OsStr>,
    {
        let output = self.output(args.clone(), Some(input))?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(self.failed(args, &output))
        }
    }

    /// Runs git and returns its output whatever its exit status.
    pub(crate) fn output<I, S>(&self, args: I, input: Option<&[u8]>) -> Result<Output, DiffError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = self.command(args);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        if input.is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = self.spawn(&mut command)?;
        if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
            // git reads all of stdin before writing much, so writing first
            // can't deadlock on a full stdout pipe for the commands used.
            stdin
                .write_all(input)
                .map_err(|source| DiffError::io("write to git", &self.workdir, source))?;
        }
        child
            .wait_with_output()
            .map_err(|source| DiffError::io("read git's output", &self.workdir, source))
    }

    /// Runs `git rev-parse --verify --quiet <rev>`: the object id, or
    /// `None` when it names nothing.
    pub(crate) fn resolve(&self, rev: &str) -> Result<Option<String>, DiffError> {
        let output = self.output(["rev-parse", "--verify", "--quiet", "--end-of-options", rev], None)?;
        if output.status.success() {
            Ok(Some(trimmed(&output.stdout)))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn spawn(&self, command: &mut Command) -> Result<Child, DiffError> {
        command.spawn().map_err(|source| DiffError::GitMissing {
            program: self.program.display().to_string(),
            source,
        })
    }

    pub(crate) fn failed<I, S>(&self, args: I, output: &Output) -> DiffError
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args: Vec<String> = args
            .into_iter()
            .map(|arg| arg.as_ref().to_string_lossy().into_owned())
            .collect();
        DiffError::Git {
            repo: self.workdir.clone(),
            args: args.join(" "),
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        }
    }
}

/// git's output as one line of text, without the line ending.
pub(crate) fn trimmed(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim_end().to_owned()
}
