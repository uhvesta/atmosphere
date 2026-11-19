//! Shared push types (`PullRequestText`, `RepoPush`).

use crate::pb;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PullRequestText {
    pub title: String,
    pub body: String,
    pub base_branch: String,
}

#[derive(Clone, Debug)]
pub struct RepoPush {
    pub repo: pb::RepoRef,
    pub branch: String,
    pub commit_message: String,
    pub open_pull_request: bool,
    pub pr: PullRequestText,
}
