//! The fake GitHub and the pull-request fixture the PR review tests share
//! (`pr_review.rs`, `pr_refresh.rs`, `pr_commits.rs`).
//!
//! GitHub is a fake `gh` (admin builds read `HUMATA2_GH`) that keeps its
//! state in files beside it: the pull requests it knows, the reviews posted
//! to it and the reviews it lists. The remotes are the world's bare
//! repositories, each pull request's head pushed to its branch and to
//! `refs/pull/<n>/head` (a fork's only to the latter), as GitHub keeps them.
//! A test file that uses it also includes `common` and `common/mcp_client.rs`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use humata2_grpc::humata::v1 as pb;
use serde_json::{Value, json};
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Request, Status};

use crate::common::{
    self, Build, Daemon, World, finished, folder, git, machine_id, repo_ref, shell_token, wait_running,
};
use crate::mcp_client::Mcp;

pub const TOKEN_HEADER: &str = "humata-session-token";

/// What the fake `gh` answers `gh auth token` with. Nothing Humata keeps may
/// ever hold it, and Humata never hands `gh` a token of its own.
pub const FAKE_TOKEN: &str = "gho_FakeTokenHumataMustNeverKeep0123456789";
/// Who the fake `gh` is signed in as: the reviewer.
pub const LOGIN: &str = "uhvesta";
/// The user's context for the review chat.
pub const CONTEXT: &str = "Mind how refunds round half cents";
/// How long a poll for something the daemon does on its own may take.
pub const SLOW: Duration = Duration::from_secs(60);

/// Index of acme/api#482 in [`Fixture::prs`].
pub const API: usize = 0;
/// acme/ledger#91.
pub const LEDGER: usize = 1;
/// acme/api#500, from a fork.
pub const FORK: usize = 2;

pub const REFUND: [&str; 10] = [
    "fn refund(cents: i64) -> i64 {",
    "    let fee = cents / 100;",
    "    let rounded = cents - fee;",
    "    // keep the sign",
    "    let signed = rounded;",
    "    log(signed);",
    "    audit(signed);",
    "    notify(signed);",
    "    signed",
    "}",
];

pub const LEDGER_LINES: [&str; 5] = [
    "struct Ledger {",
    "    entries: Vec<Entry>,",
    "}",
    "impl Ledger {}",
    "// end",
];

// ---------------------------------------------------------------------------
// The fake `gh`.

/// One call per line in `gh.log`; a token handed to it in its environment or
/// with `--with-token` is noted in `tokens.log`. `fail` (any call) and
/// `fail-post-<owner>-<name>-<n>` (posting that PR's review) hold a failure
/// mode: signed-out, rate-limited or bad-line (GitHub's 422 for a line
/// outside the diff). A repository is visible when `visible/<owner>-<name>`
/// exists; a pull request is known when `prs/<owner>-<name>-<n>.json` does.
const FAKE_GH: &str = r#"#!/bin/sh
here=$(dirname "$0")
. "$here/env"
printf '%s\n' "$*" >> "$here/gh.log"
if [ -n "${GH_TOKEN}${GITHUB_TOKEN}${GH_ENTERPRISE_TOKEN}" ]; then
  echo "a token in the environment" >> "$here/tokens.log"
fi
for arg in "$@"; do
  if [ "$arg" = "--with-token" ]; then echo "--with-token" >> "$here/tokens.log"; fi
done

unexpected() { echo "fake gh: unexpected $*" >&2; exit 1; }

fail_with() {
  case "$1" in
    signed-out)
      echo "To get started with GitHub CLI, please run:  gh auth login" >&2; exit 4 ;;
    rate-limited)
      echo "GraphQL: API rate limit exceeded for user ID 1. (HTTP 403)" >&2; exit 1 ;;
    bad-line)
      echo '{"message":"Unprocessable Entity","errors":["Pull request review thread line must be part of the diff"],"status":"422"}'
      echo "gh: Unprocessable Entity (HTTP 422)" >&2; exit 1 ;;
  esac
  echo "fake gh: unknown failure $1" >&2; exit 1
}

list() {
  printf '['
  sep=
  for f in "$1"/*.json; do
    [ -e "$f" ] || continue
    printf '%s' "$sep"; cat "$f"; sep=,
  done
  printf ']\n'
}

pr_view() {
  target=$1; shift
  repo=
  while [ $# -gt 0 ]; do
    case "$1" in
      --repo|-R) repo=$2; shift ;;
      --repo=*) repo=${1#--repo=} ;;
    esac
    shift
  done
  case "$target" in
    https://github.com/*)
      rest=${target#https://github.com/}
      repo=${rest%%/pull/*}
      target=${rest#*/pull/}
      target=${target%%[/?#]*} ;;
  esac
  owner=${repo%%/*}; name=${repo#*/}
  if [ ! -e "$here/visible/$owner-$name" ]; then
    echo "GraphQL: Could not resolve to a Repository with the name '$repo'. (repository)" >&2; exit 1
  fi
  if [ ! -e "$here/prs/$owner-$name-$target.json" ]; then
    echo "GraphQL: Could not resolve to a PullRequest with the number of $target. (repository.pullRequest)" >&2; exit 1
  fi
  cat "$here/prs/$owner-$name-$target.json"
}

api_call() {
  path=; post=; input=; jq=; graphql=; user=; prev=
  for arg in "$@"; do
    case "$prev" in
      --input) input=$arg ;;
      -X|--method) [ "$arg" = POST ] && post=1 ;;
    esac
    case "$arg" in
      --method=POST|-XPOST|--input) post=1 ;;
      --jq|-q) jq=1 ;;
      graphql) graphql=1 ;;
      user) user=1 ;;
      repos/*) path=$arg ;;
      /repos/*) path=${arg#/} ;;
    esac
    prev=$arg
  done
  if [ -n "$graphql" ]; then
    # While the hold is in place the answer waits, as a slow GitHub would
    # (bounded, so a test that forgets to lift it still ends).
    waited=0
    while [ -e "$here/graphql-hold" ] && [ "$waited" -lt 1200 ]; do
      sleep 0.05; waited=$((waited + 1))
    done
    if [ -e "$here/graphql.json" ]; then cat "$here/graphql.json"; else echo '{"data":{}}'; fi
    exit 0
  fi
  if [ -n "$user" ] && [ -z "$path" ]; then
    if [ -n "$jq" ]; then echo "$FAKE_LOGIN"; else echo "{\"login\":\"$FAKE_LOGIN\"}"; fi
    exit 0
  fi
  [ -n "$path" ] || unexpected "api $*"
  path=${path%%\?*}
  IFS=/ read -r _repos owner name _pulls number tail <<EOF
$path
EOF
  key="$owner-$name-$number"
  case "$tail" in
    reviews)
      if [ -n "$post" ]; then
        if [ -s "$here/fail-post-$key" ]; then fail_with "$(cat "$here/fail-post-$key")"; fi
        seq=$(ls "$here/posted" | wc -l | tr -d ' ')
        id=$((1001 + seq))
        if [ -z "$input" ] || [ "$input" = "-" ]; then
          cat > "$here/posted/$key.$id.json"
        else
          cat "$input" > "$here/posted/$key.$id.json"
        fi
        echo "{\"id\":$id,\"html_url\":\"https://github.com/$owner/$name/pull/$number#pullrequestreview-$id\",\"state\":\"COMMENTED\",\"user\":{\"login\":\"$FAKE_LOGIN\"}}"
      else
        list "$here/listed/$key"
      fi ;;
    reviews/*/comments)
      id=${tail#reviews/}; id=${id%/comments}
      if [ -e "$here/review-comments/$id.json" ]; then cat "$here/review-comments/$id.json"; else echo '[]'; fi ;;
    *) unexpected "api $*" ;;
  esac
}

case "$1 $2" in
  "auth token") echo "$FAKE_TOKEN"; exit 0 ;;
  "auth status")
    echo "github.com"
    echo "  Logged in to github.com account $FAKE_LOGIN (keyring)"
    echo "  Token: $FAKE_TOKEN"
    exit 0 ;;
  "api rate_limit")
    echo '{"resources":{"core":{"limit":5000,"remaining":0,"reset":4102444800},"graphql":{"limit":5000,"remaining":0,"reset":4102444800}},"rate":{"limit":5000,"remaining":0,"reset":4102444800}}'
    exit 0 ;;
esac
if [ -s "$here/fail" ]; then fail_with "$(cat "$here/fail")"; fi
case "$1" in
  pr)
    case "$2" in
      view) shift 2; pr_view "$@" ;;
      list) echo '[]' ;;
      *) unexpected "$@" ;;
    esac ;;
  api) shift; api_call "$@" ;;
  *) unexpected "$@" ;;
esac
"#;

pub struct FakeGh {
    pub dir: PathBuf,
}

impl FakeGh {
    pub fn new(dir: PathBuf) -> Self {
        for sub in ["visible", "prs", "posted", "listed", "review-comments"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        let script = dir.join("gh");
        std::fs::write(&script, FAKE_GH).unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        std::fs::write(
            dir.join("env"),
            format!("FAKE_TOKEN='{FAKE_TOKEN}'\nFAKE_LOGIN='{LOGIN}'\n"),
        )
        .unwrap();
        Self { dir }
    }

    pub fn program(&self) -> String {
        self.dir.join("gh").display().to_string()
    }

    pub fn visible(&self, owner: &str, name: &str) {
        std::fs::write(self.dir.join("visible").join(format!("{owner}-{name}")), "").unwrap();
    }

    /// Every call fails as `mode` says, or none does.
    pub fn fail(&self, mode: Option<&str>) {
        std::fs::write(self.dir.join("fail"), mode.unwrap_or_default()).unwrap();
    }

    /// Posting `pr`'s review fails as `mode` says, or doesn't.
    pub fn fail_post(&self, pr: &Pr, mode: Option<&str>) {
        std::fs::write(
            self.dir.join(format!("fail-post-{}", pr.key())),
            mode.unwrap_or_default(),
        )
        .unwrap();
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("gh.log")).unwrap_or_default()
    }

    /// How many times `gh api graphql` (the heads query) has been run.
    pub fn graphql_calls(&self) -> usize {
        self.log()
            .lines()
            .filter(|line| line.starts_with("api graphql"))
            .count()
    }

    /// While held, `gh api graphql` waits to answer until released.
    pub fn hold_graphql(&self, held: bool) {
        let gate = self.dir.join("graphql-hold");
        if held {
            std::fs::write(gate, "").unwrap();
        } else {
            let _ = std::fs::remove_file(gate);
        }
    }

    /// The reviews posted for `pr`, as `(id, body)` in the order they came.
    pub fn posted(&self, pr: &Pr) -> Vec<(u64, Value)> {
        let prefix = format!("{}.", pr.key());
        let mut posted: Vec<(u64, Value)> = std::fs::read_dir(self.dir.join("posted"))
            .unwrap()
            .filter_map(|entry| {
                let name = entry.unwrap().file_name().to_string_lossy().into_owned();
                let id = name.strip_prefix(&prefix)?.strip_suffix(".json")?.parse().ok()?;
                let text = std::fs::read_to_string(self.dir.join("posted").join(&name)).ok()?;
                Some((id, serde_json::from_str(&text).expect("a posted review is JSON")))
            })
            .collect();
        posted.sort_by_key(|(id, _)| *id);
        posted
    }

    pub fn post_count(&self) -> usize {
        std::fs::read_dir(self.dir.join("posted")).unwrap().count()
    }

    /// GitHub lists review `id` of `pr` as posted with `body` by the user
    /// `gh` is signed in as, submitted at `at` (RFC 3339), with its line
    /// comments.
    pub fn list_review(&self, pr: &Pr, id: u64, body: &Value, at: &str) {
        let state = match body["event"].as_str() {
            Some("APPROVE") => "APPROVED",
            Some("REQUEST_CHANGES") => "CHANGES_REQUESTED",
            _ => "COMMENTED",
        };
        let review = json!({
            "id": id,
            "user": { "login": LOGIN },
            "body": body["body"],
            "commit_id": body["commit_id"],
            "state": state,
            "submitted_at": at,
            "html_url": format!("{}#pullrequestreview-{id}", pr.url()),
        });
        let listed = self.dir.join("listed").join(pr.key());
        std::fs::create_dir_all(&listed).unwrap();
        std::fs::write(listed.join(format!("{id}.json")), review.to_string()).unwrap();
        let comments: Vec<Value> = body["comments"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(at, mut comment)| {
                comment["id"] = json!(id * 100 + at as u64);
                comment["pull_request_review_id"] = json!(id);
                comment["commit_id"] = body["commit_id"].clone();
                comment
            })
            .collect();
        std::fs::write(
            self.dir.join("review-comments").join(format!("{id}.json")),
            Value::Array(comments).to_string(),
        )
        .unwrap();
    }

    /// Writes what `gh pr view` and `gh api graphql` answer for `prs`.
    pub fn publish(&self, prs: &[Pr]) {
        let mut data = serde_json::Map::new();
        for pr in prs {
            let view = pr.view();
            std::fs::write(
                self.dir.join("prs").join(format!("{}.json", pr.key())),
                view.to_string(),
            )
            .unwrap();
            data.insert(
                pr.alias(),
                json!({ "pullRequest": {
                    "number": pr.number,
                    "url": pr.url(),
                    "baseRefName": pr.base,
                    "headRefName": pr.head_branch,
                    "headRefOid": view["headRefOid"],
                    "state": pr.state,
                    "commits": { "totalCount": view["commits"].as_array().map_or(0, Vec::len) },
                }}),
            );
        }
        std::fs::write(
            self.dir.join("graphql.json"),
            json!({ "data": data }).to_string(),
        )
        .unwrap();
    }

    /// GitHub's token stayed with `gh`: it was never handed one, and no
    /// file under `root` but the fake's own holds the token it knows.
    pub fn assert_token_stayed_with_gh(&self, root: &Path) {
        assert!(
            !self.dir.join("tokens.log").exists(),
            "gh was handed a token: {}",
            std::fs::read_to_string(self.dir.join("tokens.log")).unwrap_or_default()
        );
        let mut folders = vec![root.to_owned()];
        while let Some(folder) = folders.pop() {
            let Ok(entries) = std::fs::read_dir(&folder) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path == self.dir {
                    continue;
                }
                let Ok(kind) = entry.file_type() else { continue };
                if kind.is_dir() {
                    folders.push(path);
                } else if kind.is_file() {
                    let Ok(bytes) = std::fs::read(&path) else { continue };
                    assert!(
                        !bytes
                            .windows(FAKE_TOKEN.len())
                            .any(|window| window == FAKE_TOKEN.as_bytes()),
                        "{} holds GitHub's token",
                        path.display()
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Pull requests on the fake GitHub.

#[derive(Clone)]
pub struct Pr {
    pub owner: &'static str,
    pub name: &'static str,
    pub number: u32,
    pub title: &'static str,
    pub author: &'static str,
    /// The branch the PR merges into, as GitHub says it (`baseRefName`).
    pub base: &'static str,
    pub head_branch: &'static str,
    pub fork: bool,
    /// OPEN, MERGED or CLOSED, as GitHub says it.
    pub state: &'static str,
    /// A clone the head is committed in and pushed from.
    pub work: PathBuf,
}

impl Pr {
    pub fn reference(&self) -> pb::PullRequestRef {
        pb::PullRequestRef {
            repo: Some(repo_ref(self.owner, self.name)),
            number: self.number,
            ..pb::PullRequestRef::default()
        }
    }

    /// `owner/repo#n`.
    pub fn slug(&self) -> String {
        format!("{}/{}#{}", self.owner, self.name, self.number)
    }

    pub fn url(&self) -> String {
        format!("https://github.com/{}/{}/pull/{}", self.owner, self.name, self.number)
    }

    pub fn key(&self) -> String {
        format!("{}-{}-{}", self.owner, self.name, self.number)
    }

    /// The GraphQL alias the head poll asks this pull request under.
    pub fn alias(&self) -> String {
        format!("pr_{}_{}_{}", self.owner, self.name, self.number)
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect()
    }

    pub fn head(&self) -> String {
        git(&["-C", &self.work.display().to_string(), "rev-parse", "HEAD"])
    }

    pub fn line(&self, path: &str, line: usize) -> String {
        std::fs::read_to_string(self.work.join(path))
            .unwrap()
            .lines()
            .nth(line - 1)
            .unwrap()
            .to_owned()
    }

    /// The commits on the head and not on main, oldest first.
    pub fn commits(&self) -> Vec<(String, String)> {
        let work = self.work.display().to_string();
        git(&["-C", &work, "log", "--reverse", "--format=%H %s", "origin/main..HEAD"])
            .lines()
            .map(|line| {
                let (sha, subject) = line.split_once(' ').unwrap();
                (sha.to_owned(), subject.to_owned())
            })
            .collect()
    }

    /// What `gh pr view --json …` answers: every field Humata may ask for.
    pub fn view(&self) -> Value {
        let work = self.work.display().to_string();
        let (mut files, mut added, mut removed) = (0, 0, 0);
        for line in git(&["-C", &work, "diff", "--numstat", "origin/main...HEAD"]).lines() {
            let mut parts = line.split('\t');
            added += parts.next().unwrap().parse::<u32>().unwrap();
            removed += parts.next().unwrap().parse::<u32>().unwrap();
            files += 1;
        }
        let commits: Vec<Value> = self
            .commits()
            .into_iter()
            .map(|(oid, subject)| json!({ "oid": oid, "messageHeadline": subject }))
            .collect();
        json!({
            "number": self.number,
            "url": self.url(),
            "title": self.title,
            "author": { "login": self.author },
            "baseRefName": self.base,
            "headRefName": self.head_branch,
            "headRefOid": self.head(),
            "headRepositoryOwner": { "login": if self.fork { "stranger" } else { self.owner } },
            "isCrossRepository": self.fork,
            "changedFiles": files,
            "additions": added,
            "deletions": removed,
            "state": self.state,
            "mergedAt": if self.state == "MERGED" { json!("2026-11-16T10:00:00Z") } else { Value::Null },
            "commits": commits,
            "statusCheckRollup": [
                { "__typename": "CheckRun", "name": "test", "workflowName": "CI",
                  "status": "COMPLETED", "conclusion": "SUCCESS" },
                { "__typename": "CheckRun", "name": "lint", "workflowName": "CI",
                  "status": "COMPLETED", "conclusion": "FAILURE" },
            ],
        })
    }
}

/// The bare remote `owner/name` with `file` (`lines`) committed on main.
pub fn seed_main(world: &World, owner: &str, name: &str, file: &str, lines: &[&str]) -> PathBuf {
    let bare = world.remote(owner, name);
    let work = world.root.join(format!("main-{owner}-{name}"));
    let work_text = work.display().to_string();
    git(&["clone", "-q", &bare.display().to_string(), &work_text]);
    std::fs::write(work.join(file), lines.join("\n") + "\n").unwrap();
    git(&["-C", &work_text, "add", "."]);
    git(&["-C", &work_text, "commit", "-q", "-m", &format!("Add {file}")]);
    git(&["-C", &work_text, "push", "-q", "origin", "main"]);
    bare
}

/// Writes each `(path, line number, text)`, replacing that line, or adding
/// the file with `text` when the line number is 0.
pub fn edit(work: &Path, edits: &[(&str, usize, &str)]) {
    for (path, line, text) in edits {
        let file = work.join(path);
        if *line == 0 {
            std::fs::write(&file, format!("{text}\n")).unwrap();
            continue;
        }
        let mut lines: Vec<String> = std::fs::read_to_string(&file)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        lines[line - 1] = (*text).to_owned();
        std::fs::write(&file, lines.join("\n") + "\n").unwrap();
    }
}

pub fn commit(work: &Path, subject: &str, edits: &[(&str, usize, &str)]) {
    edit(work, edits);
    let work = work.display().to_string();
    git(&["-C", &work, "add", "-A"]);
    git(&["-C", &work, "commit", "-q", "-m", subject]);
}

/// Pushes the head where GitHub keeps it: `refs/pull/<n>/head`, and the
/// branch too unless the PR comes from a fork.
pub fn push_head(pr: &Pr, force: bool) {
    let work = pr.work.display().to_string();
    let mut args = vec!["-C".to_owned(), work, "push".to_owned(), "-q".to_owned()];
    if force {
        args.push("--force".to_owned());
    }
    args.push("origin".to_owned());
    args.push(format!("HEAD:refs/pull/{}/head", pr.number));
    if !pr.fork {
        args.push(format!("HEAD:refs/heads/{}", pr.head_branch));
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    git(&refs);
}

pub fn open_pr(
    world: &World,
    bare: &Path,
    template: Pr,
    subject: &str,
    edits: &[(&str, usize, &str)],
) -> Pr {
    let work = world
        .root
        .join(format!("pr-{}-{}-{}", template.owner, template.name, template.number));
    let work_text = work.display().to_string();
    git(&["clone", "-q", &bare.display().to_string(), &work_text]);
    git(&["-C", &work_text, "checkout", "-q", "-b", template.head_branch]);
    let pr = Pr { work, ..template };
    commit(&pr.work, subject, edits);
    push_head(&pr, false);
    pr
}

// ---------------------------------------------------------------------------
// The fixture.

/// Seeds `world`'s remotes with acme/api (PR #482, and #500 from a fork) and
/// acme/ledger (PR #91) and tells `gh` of them. Any daemon whose remote
/// template is the world's can clone them.
pub fn seed_pull_requests(world: &World, gh: &FakeGh) -> Vec<Pr> {
    let api = seed_main(world, "acme", "api", "refund.rs", &REFUND);
    let ledger = seed_main(world, "acme", "ledger", "ledger.rs", &LEDGER_LINES);
    let template = |owner, name, number, title, author, head_branch, fork| Pr {
        owner,
        name,
        number,
        title,
        author,
        head_branch,
        fork,
        base: "main",
        state: "OPEN",
        work: PathBuf::new(),
    };
    let prs = vec![
        open_pr(
            world,
            &api,
            template("acme", "api", 482, "Round refunds half-even", "mira", "uhvesta/fix-refund-rounding", false),
            "Round refunds half-even",
            &[
                ("refund.rs", 3, "    let rounded = round_half_even(cents - fee);"),
                ("rounding.rs", 0, "pub fn round_half_even(cents: i64) -> i64 { cents }"),
            ],
        ),
        open_pr(
            world,
            &ledger,
            template("acme", "ledger", 91, "Record refunds in the ledger", "mira", "uhvesta/refund-ledger", false),
            "Record refunds in the ledger",
            &[("ledger.rs", 2, "    entries: Vec<Entry>, // refunds too")],
        ),
        open_pr(
            world,
            &api,
            template("acme", "api", 500, "Fix a typo in refund", "stranger", "patch-1", true),
            "Fix a typo in refund",
            &[("refund.rs", 4, "    // keep the sign!")],
        ),
    ];
    gh.visible("acme", "api");
    gh.visible("acme", "ledger");
    gh.publish(&prs);
    prs
}

/// A daemon serving a world whose remotes hold acme/api with PR #482
/// (branch uhvesta/fix-refund-rounding) and PR #500 from a fork, and
/// acme/ledger with PR #91; `gh` is the fake, signed in as uhvesta.
pub struct Fixture {
    pub daemon: Daemon,
    pub world: World,
    pub gh: FakeGh,
    pub prs: Vec<Pr>,
    /// The daemon's environment at its first start, given again on restarts.
    pub env: Vec<(String, String)>,
}

/// The agent of a review session: its shell's token, an MCP client with it,
/// and the shell's attach, kept open so the token stays the shell's.
pub struct Agent {
    pub token: String,
    pub mcp: Mcp,
    pub next: u64,
    pub _attach: common::Attach,
}

impl Agent {
    pub async fn call(&mut self, tool: &str, arguments: Value) -> Value {
        self.next += 1;
        self.mcp.call(self.next, tool, arguments).await
    }

    /// A draft on `pr`'s `path` at `first..=last` on the new side, quoting
    /// the lines as the head has them; returns its comment id.
    pub async fn draft(&mut self, pr: &Pr, path: &str, first: usize, last: usize, text: &str) -> String {
        let quoted: Vec<String> = (first..=last).map(|line| pr.line(path, line)).collect();
        let reply = self
            .call(
                "review_draft",
                json!({
                    "pr": pr.slug(),
                    "path": path,
                    "lines": { "first": first, "last": last },
                    "side": "new",
                    "quoted": quoted.join("\n"),
                    "text": text,
                }),
            )
            .await;
        let result = ok(&reply);
        assert_eq!(result["status"], "draft", "{reply}");
        result["comment_id"].as_str().unwrap().to_owned()
    }
}

impl Fixture {
    pub async fn new() -> Self {
        Self::with_env(&[]).await
    }

    /// As [`Self::new`], with more environment for the daemon's first start.
    pub async fn with_env(env: &[(&str, &str)]) -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let gh = FakeGh::new(dir.path().join("gh"));
        let prs = seed_pull_requests(&world, &gh);
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        // D298 answers a check within 15 s of the last from that answer;
        // these tests check as often as they like unless they say otherwise
        // (a later entry for the same name wins).
        let mut base_env = vec![
            ("HUMATA2_GH".to_owned(), gh.program()),
            ("HUMATA2_PR_CHECK_REUSE_MS".to_owned(), "0".to_owned()),
        ];
        base_env.extend(env.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())));
        let all_env: Vec<(&str, &str)> =
            base_env.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();
        let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, &all_env).await;
        Self {
            daemon,
            world,
            gh,
            prs,
            env: base_env,
        }
    }

    /// Restarts the daemon on the same state, `gh` still the fake.
    pub async fn restart(&mut self) {
        self.restart_with(&[]).await;
    }

    /// Restarts with the first start's environment and then `env`.
    pub async fn restart_with(&mut self, env: &[(&str, &str)]) {
        let mut all_env: Vec<(&str, &str)> =
            self.env.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();
        all_env.extend_from_slice(env);
        self.daemon.restart_env(&all_env).await;
    }

    pub async fn reviews(&self) -> pb::pull_request_review_service_client::PullRequestReviewServiceClient<Channel> {
        pb::pull_request_review_service_client::PullRequestReviewServiceClient::new(self.daemon.channel().await)
    }

    pub async fn comments(&self) -> pb::comment_service_client::CommentServiceClient<Channel> {
        pb::comment_service_client::CommentServiceClient::new(self.daemon.channel().await)
    }

    pub async fn resolve(&self, reference: pb::resolve_pull_request_request::Ref) -> Result<pb::PullRequestInfo, Status> {
        self.reviews()
            .await
            .resolve_pull_request(pb::ResolvePullRequestRequest {
                r#ref: Some(reference),
            })
            .await
            .map(tonic::Response::into_inner)
    }

    /// Starts a review of the PRs at `which` (indexes into `prs`) with the
    /// user's context, and returns its session once its agent runs.
    pub async fn start_review(&self, which: &[usize]) -> pb::Session {
        let operation = self
            .reviews()
            .await
            .start_review(pb::StartReviewRequest {
                idempotency_key: None,
                prs: which.iter().map(|at| self.prs[*at].reference()).collect(),
                machine: Some(machine_id(&self.daemon).await),
                agent: None,
                model: String::new(),
                first_message: CONTEXT.to_owned(),
                push_allowed: false,
                presence: None,
            })
            .await
            .unwrap()
            .into_inner();
        let done = finished(&self.daemon, &operation).await;
        assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{done:?}");
        let Some(pb::operation::Result::Session(id)) = done.result else {
            panic!("a started review names its session: {done:?}");
        };
        wait_running(&self.daemon, &id).await
    }

    pub async fn agent(&self, session: &pb::Session) -> Agent {
        let (token, attach) = shell_token(&self.daemon, session.id.as_ref().unwrap()).await;
        let mcp = Mcp::start(&self.daemon.socket, &token).await;
        Agent {
            token,
            mcp,
            next: 0,
            _attach: attach,
        }
    }

    pub async fn pr_reviews_channels(&self) -> Vec<pb::Channel> {
        self.daemon
            .channel_service()
            .await
            .list_channels(pb::ListChannelsRequest::default())
            .await
            .unwrap()
            .into_inner()
            .channels
            .into_iter()
            .filter(|channel| channel.is_pr_reviews)
            .collect()
    }

    /// Waits until the agent of `session` has been told every fragment.
    pub async fn told(&self, session: &pb::Session, fragments: &[&str]) -> String {
        let file = self.world.conversation(&session.conversation_id);
        let deadline = Instant::now() + SLOW;
        loop {
            let said = std::fs::read_to_string(&file).unwrap_or_default();
            if fragments.iter().all(|fragment| said.contains(fragment)) {
                return said;
            }
            assert!(
                Instant::now() < deadline,
                "the agent was never told all of {fragments:?}; it was told {said:?}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub async fn list(&self, session: &pb::Session) -> Vec<pb::Comment> {
        self.comments()
            .await
            .list_comments(pb::ListCommentsRequest {
                session: session.id.clone(),
                ..pb::ListCommentsRequest::default()
            })
            .await
            .unwrap()
            .into_inner()
            .comments
    }

    /// The user opens the review's Diff, as the app does when the session
    /// opens: against the default branch, which is every fixture PR's base
    /// (PR-10). The review chat's tools work on what it shows (PR-23).
    pub async fn show_diff(&self, session: &pb::Session) -> pb::GetDiffResponse {
        pb::diff_service_client::DiffServiceClient::new(self.daemon.channel().await)
            .get_diff(pb::GetDiffRequest {
                session: session.id.clone(),
                base: Some(pb::DiffBase {
                    base: Some(pb::diff_base::Base::OriginMain(pb::OriginMain {})),
                }),
                repos: Vec::new(),
            })
            .await
            .unwrap()
            .into_inner()
    }

    /// Polls the session's comments until `done` holds for them.
    pub async fn comments_until(
        &self,
        session: &pb::Session,
        done: impl Fn(&[pb::Comment]) -> bool,
    ) -> Vec<pb::Comment> {
        let deadline = Instant::now() + SLOW;
        loop {
            let comments = self.list(session).await;
            if done(&comments) {
                return comments;
            }
            assert!(Instant::now() < deadline, "the comments never got there: {comments:#?}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub async fn plan(&self, session: &pb::Session) -> pb::SubmitPlan {
        self.reviews()
            .await
            .plan_submit(pb::PlanSubmitRequest {
                session: session.id.clone(),
            })
            .await
            .unwrap()
            .into_inner()
    }

    pub async fn submit(
        &self,
        session: &pb::Session,
        plan: &pb::SubmitPlan,
        key: &str,
        reviews: Vec<pb::ReviewSubmission>,
    ) -> Result<pb::SubmitReviewResponse, Status> {
        self.reviews()
            .await
            .submit_review(pb::SubmitReviewRequest {
                idempotency_key: Some(pb::IdempotencyKey {
                    value: key.to_owned(),
                }),
                session: session.id.clone(),
                reviews,
                plan: plan.token.clone(),
                presence: None,
            })
            .await
            .map(tonic::Response::into_inner)
    }

    pub async fn check_heads(&self, session: &pb::Session) -> pb::CheckHeadsResponse {
        self.reviews()
            .await
            .check_heads(pb::CheckHeadsRequest {
                session: session.id.clone(),
            })
            .await
            .unwrap()
            .into_inner()
    }

    /// Commits each `(subject, edits)` on the head of `prs[at]` and pushes it.
    pub fn move_head(&self, at: usize, commits: &[(&str, &[(&str, usize, &str)])]) {
        let pr = &self.prs[at];
        for (subject, edits) in commits {
            commit(&pr.work, subject, edits);
        }
        push_head(pr, false);
        self.gh.publish(&self.prs);
    }
}

/// The result of a tool call that worked.
pub fn ok(reply: &Value) -> &Value {
    assert_eq!(reply["isError"], false, "{reply}");
    &reply["structuredContent"]["result"]
}

/// The kind of a tool call's failure.
pub fn failed(reply: &Value) -> &str {
    assert_eq!(reply["isError"], true, "{reply}");
    reply["structuredContent"]["error"]["kind"].as_str().unwrap_or_default()
}

pub fn as_agent<T>(message: T, token: &str) -> Request<T> {
    let mut request = Request::new(message);
    request
        .metadata_mut()
        .insert(TOKEN_HEADER, MetadataValue::try_from(token).unwrap());
    request
}

pub fn change_of<'a>(heads: &'a pb::CheckHeadsResponse, pr: &Pr) -> Option<&'a pb::HeadChange> {
    heads.changes.iter().find(|change| change.pr == Some(pr.reference()))
}

pub fn clone_head(repo: &pb::SessionRepo) -> String {
    git(&["-C", &repo.path, "rev-parse", "HEAD"])
}

pub fn repo_of<'a>(session: &'a pb::Session, pr: &Pr) -> &'a pb::SessionRepo {
    session
        .repos
        .iter()
        .find(|repo| repo.repo == Some(repo_ref(pr.owner, pr.name)))
        .unwrap_or_else(|| panic!("the session has a clone of {}: {session:#?}", pr.slug()))
}

pub const TWO_COMMITS: [(&str, &[(&str, usize, &str)]); 2] = [
    (
        "Keep cents exact",
        &[("refund.rs", 3, "    let rounded = round_half_even_exact(cents - fee);")],
    ),
    (
        "Test rounding",
        &[("rounding_test.rs", 0, "#[test] fn halves() {}")],
    ),
];
