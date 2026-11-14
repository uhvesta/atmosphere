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

    #[error(
        "the session doesn't work in a repository named {name:?}. Read its changes to see the names of its repositories"
    )]
    UnknownRepository { name: String },

    #[error(
        "the session's copy of {repository} was put away when it was archived, so there is nothing to stage, unstage or compare there. Unarchive the session to make its copy again"
    )]
    NoCopy { repository: String },

    #[error("the session has no turn {turn}: its turns are numbered from 1 to {turns}")]
    UnknownTurn { turn: usize, turns: usize },

    #[error(
        "there's no branch or commit {branch:?} in {repository} to compare the session's copy with. Check its name with `git branch --all` there"
    )]
    UnknownBranch { branch: String, repository: String },

    #[error(
        "that change to {path} isn't there any more; the file changed since it was read. Read the changes again and pick the change from them"
    )]
    ChangeGone { path: String },

    #[error(
        "{path:?} isn't a file in the session's copy of {repository}, so nothing was changed. Name the file by its path from the repository's folder, with no `..` and no leading `/`, as the session's changes list it. Git's own `.git` isn't one of the repository's files"
    )]
    OutsideCopy { path: String, repository: String },

    #[error("{repository} has no file {path} at the end of these changes")]
    NoSuchFile { path: String, repository: String },

    #[error(
        "{repository} had no file {path} after turn {turn}, so there is no version of it to show or restore from then. Read its versions again and pick one from the list"
    )]
    NoSuchFileThen {
        path: String,
        repository: String,
        turn: usize,
    },

    #[error(
        "{branch:?} can't be a branch name. Pick another: no spaces, no `..`, and not starting with `-` or ending in `.lock`"
    )]
    BadBranch { branch: String },

    #[error(
        "{repository} already has a branch {branch:?} that isn't this session's. Pick another name, or delete that branch with `git branch -D {branch}` there if you're sure it holds nothing you want"
    )]
    BranchTaken { branch: String, repository: String },

    #[error(
        "couldn't tell which branch {repository}'s work goes into: it has no origin/HEAD and its own folder isn't on a branch. Run `git remote set-head origin --auto` in it, or check out its main branch there"
    )]
    NoBase { repository: String },

    #[error(
        "git doesn't know who you are in {directory}, so it can't commit there as you. Set your name and email with `git config --global user.name \"Your Name\"` and `git config --global user.email you@example.com` on that machine"
    )]
    NoIdentity { directory: std::path::PathBuf },
}
