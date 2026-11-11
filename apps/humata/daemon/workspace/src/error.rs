use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error(
        "couldn't run git, which Humata uses to give each session its own copy of the code. Install it (on a Mac, `xcode-select --install`) and make sure it is on the PATH humatad runs with"
    )]
    GitMissing {
        #[source]
        source: std::io::Error,
    },

    #[error(
        "`git {command}` failed in {directory}: {stderr}. Run `git status` there to see what state the repository is in"
    )]
    Git {
        directory: PathBuf,
        command: String,
        stderr: String,
    },

    #[error(
        "{path} isn't a git repository, so a session can't get its own copy of it. Run `git init` there and make a first commit, or remove it from the project"
    )]
    NotARepository { path: PathBuf },

    #[error(
        "the repository {path} has no commits yet, so there is nothing for a session's copy to start from. Make a first commit there, then send the message again"
    )]
    NoCommits { path: PathBuf },

    #[error(
        "{path} is in the way of a session's copy of {repository}, but isn't one. Move it aside or remove it, then send the message again"
    )]
    InTheWay { path: PathBuf, repository: String },

    #[error("couldn't create the folder {path}; check that it is writable and the disk has room")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't write {path}; check that it is writable and the disk has room")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
