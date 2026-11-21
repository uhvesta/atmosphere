//! The error every command returns, and its exit code.

use crate::config::ConfigError;
use crate::diff::DiffError;
use crate::gh::GhError;
use crate::git::GitError;
use crate::plan::PlanError;
use crate::restack::RestackError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Git(#[from] GitError),

    #[error(transparent)]
    Gh(#[from] GhError),

    #[error(transparent)]
    Plan(#[from] PlanError),

    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error(transparent)]
    Restack(#[from] RestackError),

    #[error(transparent)]
    Diff(#[from] DiffError),

    #[error("could not find the current directory: run stack from inside a git checkout")]
    WorkingDir {
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the pull request for topic `{topic}` (#{number}) is closed. Reopen it on GitHub to keep \
         its review, or rename the Topic trailer to open a new pull request"
    )]
    ClosedPr { topic: String, number: u64 },

    #[error(
        "topic `{topic}`'s first commit {commit} has no {sections} section. Reword it \
         (git rebase -i, `reword`) to add the sections; `stack guide --template` shows them"
    )]
    MissingSections {
        topic: String,
        commit: String,
        sections: String,
    },

    #[error("could not write a temporary file for gh at {path}: {source}. Check that the temp folder is writable")]
    TempFile {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("could not print the result as JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },

    #[error("{message}")]
    Usage {
        message: String,
        next: String,
    },
}

impl Error {
    /// The exit codes `stack --help` lists.
    pub(crate) fn exit_code(&self) -> u8 {
        match self {
            Error::Git(GitError::Conflict { .. }) => 3,
            Error::Git(GitError::MergeCommit { .. }) => 2,
            Error::Git(_) | Error::WorkingDir { .. } | Error::TempFile { .. } | Error::Json { .. } => 1,
            Error::Gh(GhError::Refused { .. }) | Error::ClosedPr { .. } => 4,
            Error::Gh(_) => 1,
            Error::Plan(_) | Error::MissingSections { .. } | Error::Usage { .. } => 2,
            Error::Config(error) => error.exit_code(),
            Error::Restack(error) => error.exit_code(),
            Error::Diff(error) => error.exit_code(),
        }
    }

    /// The command to run next, printed after the error as `Next: …`. A
    /// restack error names it in its own message.
    pub(crate) fn next_command(&self) -> Option<String> {
        let command = match self {
            Error::Git(GitError::Conflict { .. }) => {
                "git rebase -i <remote>/<trunk> to reorder or fix up the commits, then stack check".to_owned()
            }
            Error::Git(GitError::MergeCommit { .. }) => "git rebase <remote>/<trunk>, then stack check".to_owned(),
            Error::Git(GitError::Push { .. }) => "stack status, then stack upload".to_owned(),
            Error::Git(_) => "git status".to_owned(),
            Error::Gh(GhError::Refused { .. }) | Error::ClosedPr { .. } => "stack status".to_owned(),
            Error::Gh(_) => "gh auth status".to_owned(),
            Error::Plan(_) => "stack check".to_owned(),
            Error::MissingSections { .. } => "stack guide --template".to_owned(),
            Error::Config(_) => "stack config show".to_owned(),
            Error::Restack(error) if error.names_next() => return None,
            Error::Restack(error) => error.next_command(),
            Error::Diff(error) => error.next_command(),
            Error::WorkingDir { .. } => "cd into your checkout and run the same stack command".to_owned(),
            Error::TempFile { .. } | Error::Json { .. } => "the same stack command again".to_owned(),
            Error::Usage { next, .. } => next.clone(),
        };
        Some(command)
    }
}
