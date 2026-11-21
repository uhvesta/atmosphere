//! Runs the GitHub CLI. gh holds the login; stack never reads, stores or
//! prints a token. Every call goes through `Gh::run`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::de::DeserializeOwned;
use serde::Deserialize;

pub(crate) struct Gh {
    dir: PathBuf,
}

/// From `gh repo view --json nameWithOwner,url,defaultBranchRef`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Repo {
    /// `owner/name`.
    pub(crate) name_with_owner: String,
    /// `https://github.com/owner/name`.
    pub(crate) url: String,
    pub(crate) default_branch: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub(crate) enum PrState {
    Open,
    Closed,
    Merged,
}

/// A pull request, from `gh pr list --json` with `PR_FIELDS`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Pr {
    pub(crate) number: u64,
    pub(crate) url: String,
    pub(crate) state: PrState,
    pub(crate) head_ref_name: String,
    pub(crate) base_ref_name: String,
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) is_draft: bool,
    /// APPROVED, CHANGES_REQUESTED, REVIEW_REQUIRED, or empty/None.
    #[serde(default)]
    pub(crate) review_decision: Option<String>,
    /// The pull request author's login, when returned by gh.
    #[serde(default, deserialize_with = "deserialize_author")]
    pub(crate) author: String,
}

/// A native GitHub stack returned by `gh stack view --json`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct NativeStack {
    #[serde(default)]
    pub(crate) number: u64,
    #[serde(default)]
    pub(crate) base: String,
    #[serde(default)]
    pub(crate) prs: Vec<u64>,
}

fn deserialize_author<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        User { login: String },
        Login(String),
        Null,
    }
    match Option::<Raw>::deserialize(deserializer)? {
        Some(Raw::User { login }) => Ok(login),
        Some(Raw::Login(login)) => Ok(login),
        Some(Raw::Null) | None => Ok(String::new()),
    }
}

/// The `--json` fields every `gh pr list` / `gh pr view` asks for.
pub(crate) const PR_FIELDS: &str = "number,url,state,headRefName,baseRefName,title,body,isDraft,reviewDecision,author";

/// An issue comment on a pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Comment {
    pub(crate) id: u64,
    pub(crate) body: String,
    /// The login of who wrote it.
    pub(crate) author: String,
}

/// A pull request to open. The body is read from `body_file`.
pub(crate) struct NewPr<'a> {
    pub(crate) head: &'a str,
    pub(crate) base: &'a str,
    pub(crate) title: &'a str,
    pub(crate) body_file: &'a Path,
    pub(crate) draft: bool,
    pub(crate) reviewers: &'a [String],
    pub(crate) labels: &'a [String],
}

/// Changes to an open pull request; None and empty lists leave a field alone.
pub(crate) struct PrEdit<'a> {
    pub(crate) number: u64,
    pub(crate) base: Option<&'a str>,
    pub(crate) title: Option<&'a str>,
    pub(crate) body_file: Option<&'a Path>,
    pub(crate) add_reviewers: &'a [String],
    pub(crate) add_labels: &'a [String],
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GhError {
    #[error(
        "could not run `gh {args}`. Install the GitHub CLI (https://cli.github.com), make sure it \
         is on PATH, and log in with `gh auth login`"
    )]
    Spawn {
        args: String,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "`gh {args}` failed: {stderr}. If it says you are not logged in, run `gh auth login`. If it \
         names a rate limit, wait until it resets, then run the same command again: it picks up \
         where it stopped"
    )]
    Refused { args: String, stderr: String },

    #[error("could not read the JSON `gh {args}` printed")]
    Parse {
        args: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("`gh pr create` printed `{output}`, which does not end in a pull request URL")]
    PrUrl { output: String },
}

/// `gh repo view --json nameWithOwner,url,defaultBranchRef`, as gh prints it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRepo {
    name_with_owner: String,
    url: String,
    default_branch_ref: RawBranchRef,
}

#[derive(Deserialize)]
struct RawBranchRef {
    name: String,
}

#[derive(Deserialize)]
struct RawUser {
    login: String,
}

/// One element of the `gh api` comments array.
#[derive(Deserialize)]
struct RawComment {
    id: u64,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    user: Option<RawUser>,
}

impl Gh {
    pub(crate) fn new(dir: &Path) -> Gh {
        Gh {
            dir: dir.to_path_buf(),
        }
    }

    /// `gh <args>` in the checkout; its stdout. A non-zero exit is
    /// `GhError::Refused` with stderr trimmed.
    fn run(&self, args: &[&str]) -> Result<String, GhError> {
        let output = Command::new("gh")
            .current_dir(&self.dir)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("NO_COLOR", "1")
            .output()
            .map_err(|source| GhError::Spawn {
                args: args.join(" "),
                source,
            })?;
        if !output.status.success() {
            return Err(GhError::Refused {
                args: args.join(" "),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Runs `gh <args>` and parses its stdout as one JSON value.
    fn json<T: DeserializeOwned>(&self, args: &[&str]) -> Result<T, GhError> {
        let output = self.run(args)?;
        serde_json::from_str(&output).map_err(|source| GhError::Parse {
            args: args.join(" "),
            source,
        })
    }

    /// `gh repo view --json nameWithOwner,url,defaultBranchRef`.
    pub(crate) fn repo(&self) -> Result<Repo, GhError> {
        let raw: RawRepo =
            self.json(&["repo", "view", "--json", "nameWithOwner,url,defaultBranchRef"])?;
        Ok(Repo {
            name_with_owner: raw.name_with_owner,
            url: raw.url,
            default_branch: raw.default_branch_ref.name,
        })
    }

    /// The logged-in user, from `gh api user` (field `login`).
    pub(crate) fn login(&self) -> Result<String, GhError> {
        let user: RawUser = self.json(&["api", "user"])?;
        Ok(user.login)
    }

    /// The newest pull request whose head is `branch`, in any state:
    /// `gh pr list --head <branch> --state all --json <PR_FIELDS>`, first item.
    pub(crate) fn pr_for_head(&self, branch: &str) -> Result<Option<Pr>, GhError> {
        let prs: Vec<Pr> = self.json(&[
            "pr",
            "list",
            "--head",
            branch,
            "--state",
            "all",
            "--limit",
            "20",
            "--json",
            PR_FIELDS,
        ])?;
        Ok(prs.into_iter().max_by_key(|pr| pr.number))
    }

    /// Your open pull requests: `gh pr list --author @me --state open --limit 200 --json <PR_FIELDS>`.
    pub(crate) fn my_open_prs(&self) -> Result<Vec<Pr>, GhError> {
        self.json(&[
            "pr",
            "list",
            "--author",
            "@me",
            "--state",
            "open",
            "--limit",
            "200",
            "--json",
            PR_FIELDS,
        ])
    }

    /// All open pull requests in the repository: `gh pr list --state open --limit 200 --json <PR_FIELDS>`.
    pub(crate) fn open_prs(&self) -> Result<Vec<Pr>, GhError> {
        self.json(&[
            "pr",
            "list",
            "--state",
            "open",
            "--limit",
            "200",
            "--json",
            PR_FIELDS,
        ])
    }

    /// One pull request by number, URL, or branch: `gh pr view <target> --json <PR_FIELDS>`.
    pub(crate) fn pr_view(&self, target: &str) -> Result<Pr, GhError> {
        self.json(&["pr", "view", target, "--json", PR_FIELDS])
    }

    /// Marks a draft pull request ready for review: `gh pr ready <number>`.
    pub(crate) fn pr_ready(&self, number: u64) -> Result<(), GhError> {
        let number = number.to_string();
        self.run(&["pr", "ready", number.as_str()])?;
        Ok(())
    }

    /// `gh pr create --head --base --title --body-file [--draft] [--reviewer a,b] [--label x,y]`.
    /// Returns the new pull request's number and URL (gh prints the URL; the
    /// number is its last path segment).
    pub(crate) fn create_pr(&self, pr: &NewPr<'_>) -> Result<(u64, String), GhError> {
        let body_file = pr.body_file.to_string_lossy();
        let reviewers = pr.reviewers.join(",");
        let labels = pr.labels.join(",");
        let mut args = vec![
            "pr",
            "create",
            "--head",
            pr.head,
            "--base",
            pr.base,
            "--title",
            pr.title,
            "--body-file",
            &*body_file,
        ];
        if pr.draft {
            args.push("--draft");
        }
        if !pr.reviewers.is_empty() {
            args.push("--reviewer");
            args.push(reviewers.as_str());
        }
        if !pr.labels.is_empty() {
            args.push("--label");
            args.push(labels.as_str());
        }
        let output = self.run(&args)?;
        parse_pr_url(&output)
    }

    /// `gh pr edit <number> [--base] [--title] [--body-file] [--add-reviewer a,b] [--add-label x,y]`.
    /// Does nothing when the edit changes nothing.
    pub(crate) fn edit_pr(&self, edit: &PrEdit<'_>) -> Result<(), GhError> {
        let number = edit.number.to_string();
        let body_file = edit.body_file.map(|path| path.to_string_lossy());
        let reviewers = edit.add_reviewers.join(",");
        let labels = edit.add_labels.join(",");
        let mut args = vec!["pr", "edit", number.as_str()];
        let fixed_len = args.len();
        if let Some(base) = edit.base {
            args.push("--base");
            args.push(base);
        }
        if let Some(title) = edit.title {
            args.push("--title");
            args.push(title);
        }
        if let Some(file) = &body_file {
            args.push("--body-file");
            args.push(file);
        }
        if !edit.add_reviewers.is_empty() {
            args.push("--add-reviewer");
            args.push(reviewers.as_str());
        }
        if !edit.add_labels.is_empty() {
            args.push("--add-label");
            args.push(labels.as_str());
        }
        if args.len() == fixed_len {
            return Ok(());
        }
        self.run(&args)?;
        Ok(())
    }

    /// `gh api --paginate repos/{owner}/{repo}/issues/<number>/comments`:
    /// objects with `id`, `body` and `user.login`. gh fills in `{owner}/{repo}`.
    pub(crate) fn comments(&self, number: u64) -> Result<Vec<Comment>, GhError> {
        let path = format!("repos/{{owner}}/{{repo}}/issues/{number}/comments");
        let args = ["api", "--paginate", path.as_str()];
        let output = self.run(&args)?;
        parse_comments(&args.join(" "), &output)
    }

    /// `gh api -X POST repos/{owner}/{repo}/issues/<number>/comments -F body=@<file>`.
    pub(crate) fn add_comment(&self, number: u64, body_file: &Path) -> Result<(), GhError> {
        let path = format!("repos/{{owner}}/{{repo}}/issues/{number}/comments");
        let body = format!("body=@{}", body_file.to_string_lossy());
        self.run(&["api", "-X", "POST", path.as_str(), "-F", body.as_str()])?;
        Ok(())
    }

    /// `gh api -X PATCH repos/{owner}/{repo}/issues/comments/<id> -F body=@<file>`.
    pub(crate) fn edit_comment(&self, id: u64, body_file: &Path) -> Result<(), GhError> {
        let path = format!("repos/{{owner}}/{{repo}}/issues/comments/{id}");
        let body = format!("body=@{}", body_file.to_string_lossy());
        self.run(&["api", "-X", "PATCH", path.as_str(), "-F", body.as_str()])?;
        Ok(())
    }

    /// Whether the GitHub CLI's stack extension is installed: `gh stack --help` exits 0.
    pub(crate) fn has_native_stacks(&self) -> bool {
        self.run(&["stack", "--help"]).is_ok()
    }

    /// `gh stack link --base <trunk> <url>...`, bottom of the stack first.
    pub(crate) fn link_stack(&self, trunk: &str, urls: &[String]) -> Result<(), GhError> {
        let mut args = vec!["stack", "link", "--base", trunk];
        args.extend(urls.iter().map(String::as_str));
        self.run(&args)?;
        Ok(())
    }

    /// Native GitHub stacks from `gh stack view --json`, when the extension is
    /// installed and returns JSON. Returns `Ok(None)` when unavailable.
    pub(crate) fn native_stack_view(&self) -> Result<Option<Vec<NativeStack>>, GhError> {
        if !self.has_native_stacks() {
            return Ok(None);
        }
        let Ok(output) = self.run(&["stack", "view", "--json"]) else {
            return Ok(None);
        };
        let trimmed = output.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        if let Ok(list) = serde_json::from_str::<Vec<NativeStack>>(trimmed) {
            return Ok(Some(list));
        }
        if let Ok(single) = serde_json::from_str::<NativeStack>(trimmed) {
            return Ok(Some(vec![single]));
        }
        Ok(None)
    }
}

/// Reads the pull request URL from `gh pr create` output: the last non-empty
/// line must end in `/pull/<number>`.
fn parse_pr_url(output: &str) -> Result<(u64, String), GhError> {
    let line = output
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("");
    let number = line
        .rsplit_once('/')
        .and_then(|(_, segment)| segment.parse::<u64>().ok());
    match number {
        Some(number) if line.contains("/pull/") => Ok((number, line.to_owned())),
        _ => Err(GhError::PrUrl {
            output: output.trim().to_owned(),
        }),
    }
}

/// Parses one or more JSON arrays of comments printed back to back, as
/// `gh api --paginate` prints them, and keeps their order.
fn parse_comments(args: &str, output: &str) -> Result<Vec<Comment>, GhError> {
    let mut comments = Vec::new();
    for batch in serde_json::Deserializer::from_str(output).into_iter::<Vec<RawComment>>() {
        let batch = batch.map_err(|source| GhError::Parse {
            args: args.to_owned(),
            source,
        })?;
        comments.extend(batch.into_iter().map(|raw| Comment {
            id: raw.id,
            body: raw.body.unwrap_or_default(),
            author: raw.user.map(|user| user.login).unwrap_or_default(),
        }));
    }
    Ok(comments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pr_url_comes_from_a_plain_url_line() {
        assert_eq!(
            parse_pr_url("https://github.com/o/r/pull/12\n").ok(),
            Some((12, "https://github.com/o/r/pull/12".to_owned()))
        );
    }

    #[test]
    fn pr_url_is_the_last_non_empty_line_after_other_output() {
        assert_eq!(
            parse_pr_url("Warning: something\nhttps://github.com/o/r/pull/12\n").ok(),
            Some((12, "https://github.com/o/r/pull/12".to_owned()))
        );
    }

    #[test]
    fn output_without_a_pull_request_url_is_an_error() {
        assert!(matches!(
            parse_pr_url("created\n"),
            Err(GhError::PrUrl { .. })
        ));
    }

    #[test]
    fn non_numeric_last_segment_is_an_error() {
        assert!(matches!(
            parse_pr_url("https://github.com/o/r/pull/abc"),
            Err(GhError::PrUrl { .. })
        ));
    }

    #[test]
    fn one_comments_array_parses_into_comments() {
        let output = r#"[{"id":1,"body":"hi","user":{"login":"ann"}}]"#;
        assert_eq!(
            parse_comments("api", output).ok(),
            Some(vec![Comment {
                id: 1,
                body: "hi".to_owned(),
                author: "ann".to_owned(),
            }])
        );
    }

    #[test]
    fn concatenated_comment_arrays_keep_their_order() {
        let output = concat!(
            r#"[{"id":1,"body":"a","user":{"login":"ann"}}]"#,
            r#"[{"id":2,"body":"b","user":{"login":"bo"}}]"#
        );
        let ids: Option<Vec<u64>> = parse_comments("api", output)
            .ok()
            .map(|comments| comments.iter().map(|comment| comment.id).collect());
        assert_eq!(ids, Some(vec![1, 2]));
    }

    #[test]
    fn empty_or_blank_output_has_no_comments() {
        assert_eq!(parse_comments("api", "").ok(), Some(Vec::new()));
        assert_eq!(parse_comments("api", " \n ").ok(), Some(Vec::new()));
    }

    #[test]
    fn null_body_and_missing_user_become_empty_fields() {
        let output = r#"[{"id":3,"body":null}]"#;
        assert_eq!(
            parse_comments("api", output).ok(),
            Some(vec![Comment {
                id: 3,
                body: String::new(),
                author: String::new(),
            }])
        );
    }

    #[test]
    fn invalid_json_is_a_parse_error() {
        assert!(matches!(
            parse_comments("api", "[{\"id\":"),
            Err(GhError::Parse { .. })
        ));
    }

    #[test]
    fn pr_reads_gh_json_with_merged_state_and_no_body() {
        let json = r#"{"number":5,"url":"https://github.com/o/r/pull/5","state":"MERGED","headRefName":"feat","baseRefName":"main","title":"Add","isDraft":false,"reviewDecision":""}"#;
        let pr = serde_json::from_str::<Pr>(json).ok();
        assert_eq!(pr.as_ref().map(|pr| pr.state), Some(PrState::Merged));
        assert_eq!(
            pr.as_ref().map(|pr| pr.review_decision.as_deref()),
            Some(Some(""))
        );
        assert_eq!(pr.map(|pr| pr.body), Some(String::new()));
    }
}
