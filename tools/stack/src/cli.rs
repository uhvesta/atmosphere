//! The command line. The help text here is the manual: an agent with only
//! `stack --help`, `stack <command> --help` and `stack guide` can build a
//! stack.

use clap::{Args, Parser, Subcommand};

const ABOUT: &str = "Turn the commits on your branch into a linked stack of GitHub pull requests.";

const LONG_ABOUT: &str = "\
Turn the commits on your branch into a linked stack of GitHub pull requests.

Each commit says which pull request it belongs to with trailers in the last
paragraph of its message:

    storage: compress the journal

    ## Summary
    Journal segments are written with zstd.

    ## Why
    Journals were 40% of the disk.

    ## Testing
    bazel test //daemon/store/...

    Topic: compress-journal
    Relative: zstd-codec

All commits with the same Topic make one pull request, on the branch
<prefix>/stack/<topic> (the prefix is your GitHub login unless stack.toml or
--prefix sets one). Branch names never change when you reorder topics, so a
pull request stays open through any restack. Relative names the topic this one
builds on: its pull request targets that topic's branch, so the pull requests
form a chain (or a tree). A topic without Relative targets the trunk. Commits
without a Topic stay local.

The pull request's title is the subject of the topic's first commit; its
description is that commit's body laid out in the PR template, followed by the
whole stack with this pull request marked. Each pull request also carries one
comment with a table of every revision you uploaded; each revision's head
and base stay pinned on the remote under refs/stack/<prefix>/<topic>/r<N>.

The usual loop:
    stack check            find mistakes locally, before anything is pushed
    stack upload --dry-run see what would be pushed and opened
    stack upload           push the branches, open or update the pull requests
    stack status           see each pull request and whether it needs an upload
To revise an earlier topic:
    stack restack edit --topic codec   stop at that topic's last commit
    git commit --amend                 change it (or add commits)
    stack restack continue             replay the later topics onto it
    stack upload --note \"what changed\"
    stack diff codec                   what changed since the last upload,
                                       base changes left out

Settings: every default is compiled in; ~/.humata2/config/stack.toml (or
$HUMATA2_HOME/config/stack.toml) can change them. `stack config show` prints
each setting and where it came from. Secrets never go there: stack uses the
GitHub CLI's login (`gh auth login`).

Run `stack guide` for the agent manual and `stack commands --json` for every
command and flag as data.";

/// Exit codes, also listed in `stack --help`.
const AFTER_HELP: &str = "\
Exit codes:
  0  done
  1  git or gh could not run, or failed
  2  a usage, trailer, plan or template problem: fix the commits and run again
  3  a commit does not apply where its topic puts it (a rebuild or restack
     conflict): resolve it, then run the command the error names
  4  GitHub refused, or a pull request's state blocks the upload
  5  a restack's --exec command failed at a topic

Every error ends with a `Next:` line: the command to run next.

Run `stack <command> --help` for each command's options.";

#[derive(Debug, Parser)]
#[command(
    name = "stack",
    about = ABOUT,
    long_about = LONG_ABOUT,
    after_help = AFTER_HELP
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Check the trailers, the topics and their PR descriptions locally, without
    /// pushing or calling GitHub.
    #[command(long_about = "\
Check the trailers, the topics and their PR descriptions locally, without
pushing or calling GitHub.

Reads the commits between the trunk (as last fetched) and HEAD, groups them by
Topic, checks every Relative names a topic on this branch with no cycles, checks
each topic's first commit has the template's required sections, and rebuilds
each topic onto its relative to find conflicts early. Prints the planned
stack: each topic, its base and its commits, and the commits without a Topic.")]
    Check(CheckArgs),

    /// Push each topic's branch and open or update its pull request.
    #[command(long_about = "\
Push each topic's branch and open or update its pull request.

Fetches the trunk, rebuilds each topic onto its relative's branch (or the
trunk), pushes every changed branch in one atomic push, then opens or updates
one pull request per topic with the right base, title, description, reviewers
and labels. Each pull request's history comment gets a row for every upload
that changed its head. When the GitHub CLI's stack extension is installed, the
chain is also linked as a native GitHub stack; if that fails the pull requests
still chain by base branch.

Running it again after an amend updates the same pull requests in place. A
topic whose pull request was merged is skipped; rebase onto the trunk to drop
its commits. A topic whose pull request was closed stops the upload: reopen it
on GitHub, or rename the Topic to open a new one.")]
    Upload(UploadArgs),

    /// Show each topic's pull request and whether it needs an upload.
    #[command(long_about = "\
Show each topic's pull request and whether it needs an upload.

Draws the stack as a tree, each topic under the topic its Relative names. For
every topic: its branch, pull request number and URL, state (open, draft,
merged, closed), base, and `up to date` or `needs upload`. Shows a restack in
progress and the command that resumes it. Also lists your open stack pull requests that no local topic
uses any more (orphaned), with the command to close them if they are not
needed.")]
    Status(StatusArgs),

    /// Print how to write commits that become good pull requests.
    #[command(long_about = "\
Print how to write commits that become good pull requests: the trailers, the
title, the description sections and the revision loop. With --template, print
the PR template in use and where it comes from (stack.toml's `template`, or
the built-in one).")]
    Guide(GuideArgs),

    /// Edit an earlier topic, then replay the topics above it.
    #[command(subcommand, long_about = "\
Edit an earlier topic, then replay the topics above it. The git analogue is
`git rebase -i` with `edit`, `--continue`, `--skip`, `--abort` and `--exec`.

    stack restack edit --topic codec     stop at codec's last commit, checked out
    git commit --amend                   change it, or add commits
    stack restack continue               replay every later commit onto it
    stack restack continue --step        ...stopping after each topic to build or test
    stack restack continue --exec \"bazel test //...\"
                                         ...running a command at each topic, stopping if it fails
    stack restack skip                   drop the commit that conflicts and go on
    stack restack abort                  put the branch back as it was

The restack's state lives in the repository's git folder, so it survives a
crash; `stack status` shows it. After it finishes, `stack upload` pushes the
changed topics, one history row per changed pull request.")]
    Restack(RestackCommand),

    /// Show what changed between two uploaded revisions of a topic.
    #[command(long_about = "\
Show what changed between two uploaded revisions of a topic, with base
changes left out: `git range-diff` between the revisions' pinned commits.

Every upload pins each changed pull request's head and base under
refs/stack/<prefix>/<topic>/r<N> (and r<N>-base) on the remote, so old
revisions stay fetchable after a force-push. This fetches the pins it needs.
Revisions are the numbers in the pull request's history table (`3` or `r3`);
the default is the last two. A history row marked `rebase` names the exact
command for its range.")]
    Diff(DiffArgs),

    /// Show or check the settings: compiled-in defaults and stack.toml.
    #[command(subcommand)]
    Config(ConfigCommand),

    /// List every command with its arguments and a one-line purpose.
    #[command(long_about = "\
List every command with its arguments and a one-line purpose. With --json the
list is data an agent can read: each command's path, purpose and arguments
with their names, value names, defaults and help.")]
    Commands(CommandsArgs),
}

#[derive(Debug, Subcommand)]
pub(crate) enum RestackCommand {
    /// Stop at an earlier topic or commit, checked out, to amend it.
    #[command(long_about = "\
Stop at an earlier topic or commit, checked out, to amend it.

With --topic the stop is that topic's last commit, so new commits you add
(with the same Topic trailer) join that topic. With --commit it is that
commit. Change files, then `git commit --amend` or add commits, and run
`stack restack continue`. Needs a clean working tree.")]
    Edit(EditArgs),

    /// Replay the rest of the stack onto your amended commit.
    #[command(long_about = "\
Replay the rest of the stack onto your amended commit.

Without --step it runs until every later commit is replayed or a commit
conflicts. With --step it also stops after each later topic so you can build
or test, then `stack restack continue` again. --step and --exec given here
replace the ones given before for the rest of the restack.")]
    Continue(ReplayArgs),

    /// Drop the commit that conflicts and keep replaying.
    Skip(ReplayArgs),

    /// Cancel the restack and put the branch back as it was.
    Abort,
}

#[derive(Debug, Args)]
#[group(id = "edit_target", required = true, multiple = false)]
pub(crate) struct EditTarget {
    /// The topic to edit: the restack stops at its last commit.
    #[arg(long, value_name = "NAME")]
    pub(crate) topic: Option<String>,

    /// The commit to edit, by sha (abbreviated is fine).
    #[arg(long, value_name = "SHA")]
    pub(crate) commit: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct EditArgs {
    #[command(flatten)]
    pub(crate) target: Target,

    #[command(flatten)]
    pub(crate) edit: EditTarget,

    #[command(flatten)]
    pub(crate) replay: ReplayArgs,
}

#[derive(Debug, Args)]
pub(crate) struct ReplayArgs {
    /// Stop after each later topic is replayed, to build or test there.
    #[arg(long)]
    pub(crate) step: bool,

    /// Run this shell command after each topic is replayed, like
    /// `git rebase --exec`; the restack stops where it fails.
    #[arg(long, value_name = "COMMAND")]
    pub(crate) exec: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct DiffArgs {
    /// The topic whose revisions to compare.
    #[arg(value_name = "TOPIC")]
    pub(crate) topic: String,

    /// The older revision. Default: the one before the newest.
    #[arg(value_name = "FROM_REV")]
    pub(crate) from: Option<String>,

    /// The newer revision. Default: the newest.
    #[arg(value_name = "TO_REV", requires = "from")]
    pub(crate) to: Option<String>,

    /// The git remote holding the pins. Default: stack.toml's `remote`, else
    /// `origin`.
    #[arg(long, value_name = "NAME")]
    pub(crate) remote: Option<String>,

    /// The branch prefix the topic was uploaded under. Default: stack.toml's
    /// `prefix`, else the only prefix that has pins for this topic.
    #[arg(long, value_name = "PREFIX")]
    pub(crate) prefix: Option<String>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ConfigCommand {
    /// Print every setting, its value and where it came from.
    #[command(long_about = "\
Print every setting, its value and where it came from: `default` (compiled in)
or `stack.toml`. The file is <humata home>/config/stack.toml, where the humata
home is $HUMATA2_HOME, else ~/.humata2. A missing file means every default.")]
    Show(JsonArg),
}

#[derive(Debug, Args)]
pub(crate) struct JsonArg {
    /// Print JSON instead of text.
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Args)]
pub(crate) struct CommandsArgs {
    /// Print JSON instead of text.
    #[arg(long)]
    pub(crate) json: bool,
}

/// Where the trunk and the remote come from, shared by every command that
/// reads the branch.
#[derive(Debug, Args)]
pub(crate) struct Target {
    /// The branch the stack goes into. Default: stack.toml's `trunk`, else
    /// the repository's default branch on GitHub (for local commands: the
    /// remote's HEAD, else `main`).
    #[arg(long, value_name = "BRANCH", visible_alias = "base")]
    pub(crate) trunk: Option<String>,

    /// The git remote to fetch the trunk from and push the branches to.
    /// Default: stack.toml's `remote`, else `origin`.
    #[arg(long, value_name = "NAME")]
    pub(crate) remote: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct CheckArgs {
    #[command(flatten)]
    pub(crate) target: Target,

    /// Print the plan as JSON instead of text.
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Args)]
pub(crate) struct UploadArgs {
    #[command(flatten)]
    pub(crate) target: Target,

    /// Print what would be pushed and which pull requests would be opened or
    /// updated, and change nothing.
    #[arg(long)]
    pub(crate) dry_run: bool,

    /// Print the result as JSON: one object per topic with its pull request
    /// and what was done.
    #[arg(long)]
    pub(crate) json: bool,

    /// Open new pull requests as drafts (a topic's `Draft: true` trailer does
    /// the same for one topic; stack.toml's `draft = true` for every run).
    /// Existing pull requests keep their state.
    #[arg(long)]
    pub(crate) draft: bool,

    /// Upload only this topic and the topics it is relative to. Repeat for
    /// more topics.
    #[arg(long = "topic", value_name = "TOPIC")]
    pub(crate) topics: Vec<String>,

    /// A note for reviewers, added to this revision's row in each changed pull
    /// request's history, such as "addressed review: renamed Codec".
    #[arg(long, value_name = "TEXT")]
    pub(crate) note: Option<String>,

    /// Don't link the pull requests as a native GitHub stack, even when the
    /// stack extension of the GitHub CLI is installed (stack.toml:
    /// `native_stack = false`).
    #[arg(long)]
    pub(crate) no_native_stack: bool,

    /// Replace pull request descriptions that were edited on GitHub. Without
    /// it, an edited description is kept and a warning names the pull request
    /// (stack.toml: `overwrite_body = true`).
    #[arg(long)]
    pub(crate) overwrite_body: bool,

    /// The first part of new branch names, `<PREFIX>/stack/<topic>`, for this
    /// run. Default: stack.toml's `prefix`, else your GitHub login. A topic
    /// whose pull request is open keeps the branch it has.
    #[arg(long, value_name = "PREFIX")]
    pub(crate) prefix: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct StatusArgs {
    #[command(flatten)]
    pub(crate) target: Target,

    /// Print the status as JSON instead of text.
    #[arg(long)]
    pub(crate) json: bool,

    /// Read the stack without the network: no fetch and no GitHub calls, so
    /// no pull requests and no GitHub login are needed. The trunk comes from
    /// --trunk, stack.toml, or the remote's HEAD as last fetched.
    #[arg(long)]
    pub(crate) offline: bool,
}

#[derive(Debug, Args)]
pub(crate) struct GuideArgs {
    /// Print the PR template in use and its path instead of the guide.
    #[arg(long)]
    pub(crate) template: bool,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::Cli;

    #[test]
    fn the_command_line_is_consistent() {
        Cli::command().debug_assert();
    }
}
