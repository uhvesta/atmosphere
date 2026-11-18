//! A stand-in for the GitHub CLI in stack's tests. It answers the calls
//! stack makes from a JSON state file at $FAKE_GH_STATE and records every
//! call there, so a test can set GitHub up, run stack, and read back what
//! stack did. It never reaches the network.
//!
//! State: {"login", "repo" (owner/name), "default_branch", "native_stack"
//! ("absent" | "linked" | "refused"), "next_number", "next_comment_id",
//! "prs": [{"number", "url", "head", "base", "title", "body", "state"
//! (OPEN | CLOSED | MERGED), "draft", "reviewers", "labels", "comments":
//! [{"id", "body", "login"}]}], "stacks": [[url, ...]], "calls": [[arg, ...]]}.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Value, json};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = std::env::var_os("FAKE_GH_STATE").map(PathBuf::from) else {
        eprintln!("fake gh: FAKE_GH_STATE is not set");
        return ExitCode::from(2);
    };
    let mut state = match read_state(&path) {
        Ok(state) => state,
        Err(message) => {
            eprintln!("fake gh: {message}");
            return ExitCode::from(2);
        }
    };
    if let Some(calls) = state["calls"].as_array_mut() {
        calls.push(json!(args));
    }
    let result = answer(&mut state, &args);
    if let Err(message) = write_state(&path, &state) {
        eprintln!("fake gh: {message}");
        return ExitCode::from(2);
    }
    match result {
        Ok(stdout) => {
            print!("{stdout}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(1)
        }
    }
}

fn read_state(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("parse {}: {error}", path.display()))
}

fn write_state(path: &Path, state: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(state).map_err(|error| error.to_string())?;
    std::fs::write(path, text).map_err(|error| format!("write {}: {error}", path.display()))
}

fn answer(state: &mut Value, args: &[String]) -> Result<String, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["repo", "view", ..] => Ok(repo_view(state)),
        ["api", "user", ..] => Ok(json!({"login": state["login"]}).to_string()),
        ["pr", "list", rest @ ..] => pr_list(state, rest),
        ["pr", "create", rest @ ..] => pr_create(state, rest),
        ["pr", "edit", number, rest @ ..] => pr_edit(state, number, rest),
        ["api", rest @ ..] => api(state, rest),
        ["stack", "--help"] => native(state).map(|()| "Usage: gh stack <command>\n".to_owned()),
        ["stack", "link", rest @ ..] => stack_link(state, rest),
        _ => Err(format!("fake gh: unexpected call: gh {}", args.join(" "))),
    }
}

fn repo_view(state: &Value) -> String {
    let repo = state["repo"].as_str().unwrap_or("o/r");
    json!({
        "nameWithOwner": repo,
        "url": format!("https://github.com/{repo}"),
        "defaultBranchRef": {"name": state["default_branch"]},
    })
    .to_string()
}

/// The value after `flag`, if any.
fn flag<'a>(rest: &[&'a str], flag: &str) -> Option<&'a str> {
    rest.iter().position(|word| *word == flag).and_then(|at| rest.get(at + 1)).copied()
}

fn has(rest: &[&str], flag: &str) -> bool {
    rest.contains(&flag)
}

fn list(value: Option<&str>) -> Vec<Value> {
    value
        .map(|value| value.split(',').filter(|item| !item.is_empty()).map(|item| json!(item)).collect())
        .unwrap_or_default()
}

fn pr_json(pr: &Value) -> Value {
    json!({
        "number": pr["number"],
        "url": pr["url"],
        "state": pr["state"],
        "headRefName": pr["head"],
        "baseRefName": pr["base"],
        "title": pr["title"],
        "body": pr["body"],
        "isDraft": pr["draft"],
        "reviewDecision": "",
    })
}

fn prs(state: &mut Value) -> Result<&mut Vec<Value>, String> {
    state["prs"].as_array_mut().ok_or_else(|| "fake gh: state has no prs list".to_owned())
}

fn pr_list(state: &mut Value, rest: &[&str]) -> Result<String, String> {
    if flag(rest, "--json").is_none() {
        return Err("fake gh: pr list without --json".to_owned());
    }
    let head = flag(rest, "--head");
    let mine = flag(rest, "--author") == Some("@me");
    let state_filter = flag(rest, "--state").unwrap_or("open");
    let mut found: Vec<Value> = prs(state)?
        .iter()
        .filter(|pr| head.is_none_or(|head| pr["head"] == head))
        .filter(|pr| state_filter == "all" || pr["state"] == state_filter.to_uppercase())
        .filter(|_| mine || head.is_some())
        .map(pr_json)
        .collect();
    // Newest first, as gh lists them.
    found.reverse();
    Ok(Value::Array(found).to_string())
}

fn body_from_file(rest: &[&str]) -> Result<Option<String>, String> {
    match flag(rest, "--body-file") {
        Some(path) => std::fs::read_to_string(path)
            .map(Some)
            .map_err(|error| format!("fake gh: read body file {path}: {error}")),
        None => Ok(None),
    }
}

fn pr_create(state: &mut Value, rest: &[&str]) -> Result<String, String> {
    let head = flag(rest, "--head").ok_or("fake gh: pr create without --head")?;
    let base = flag(rest, "--base").ok_or("fake gh: pr create without --base")?;
    let title = flag(rest, "--title").ok_or("fake gh: pr create without --title")?;
    let body = body_from_file(rest)?.ok_or("fake gh: pr create without --body-file")?;
    if prs(state)?.iter().any(|pr| pr["head"] == head && pr["state"] == "OPEN") {
        return Err(format!("a pull request for branch \"{head}\" already exists"));
    }
    let number = state["next_number"].as_u64().unwrap_or(1);
    state["next_number"] = json!(number + 1);
    let repo = state["repo"].as_str().unwrap_or("o/r").to_owned();
    let url = format!("https://github.com/{repo}/pull/{number}");
    let pr = json!({
        "number": number,
        "url": url,
        "head": head,
        "base": base,
        "title": title,
        "body": body,
        "state": "OPEN",
        "draft": has(rest, "--draft"),
        "reviewers": list(flag(rest, "--reviewer")),
        "labels": list(flag(rest, "--label")),
        "comments": [],
    });
    prs(state)?.push(pr);
    Ok(format!("{url}\n"))
}

fn find_pr<'a>(state: &'a mut Value, number: &str) -> Result<&'a mut Value, String> {
    let number: u64 = number.parse().map_err(|_| format!("fake gh: not a PR number: {number}"))?;
    prs(state)?
        .iter_mut()
        .find(|pr| pr["number"] == number)
        .ok_or_else(|| format!("no pull requests found for {number}"))
}

fn pr_edit(state: &mut Value, number: &str, rest: &[&str]) -> Result<String, String> {
    let body = body_from_file(rest)?;
    let pr = find_pr(state, number)?;
    if pr["state"] != "OPEN" {
        return Err(format!("pull request #{number} is not open"));
    }
    if let Some(base) = flag(rest, "--base") {
        pr["base"] = json!(base);
    }
    if let Some(title) = flag(rest, "--title") {
        pr["title"] = json!(title);
    }
    if let Some(body) = body {
        pr["body"] = json!(body);
    }
    for (option, field) in [("--add-reviewer", "reviewers"), ("--add-label", "labels")] {
        for item in list(flag(rest, option)) {
            if let Some(items) = pr[field].as_array_mut()
                && !items.contains(&item)
            {
                items.push(item);
            }
        }
    }
    Ok(format!("{}\n", pr["url"].as_str().unwrap_or_default()))
}

/// `gh api [--paginate] [-X METHOD] <path> [-F body=@file]`.
fn api(state: &mut Value, rest: &[&str]) -> Result<String, String> {
    let method = flag(rest, "-X").unwrap_or("GET");
    let path = rest
        .iter()
        .find(|word| word.starts_with("repos/"))
        .ok_or("fake gh: api call without a repos/ path")?;
    let parts: Vec<&str> = path.split('/').collect();
    let body = match flag(rest, "-F") {
        Some(field) => {
            let file = field.strip_prefix("body=@").ok_or("fake gh: -F must be body=@<file>")?;
            Some(std::fs::read_to_string(file).map_err(|error| format!("fake gh: read {file}: {error}"))?)
        }
        None => None,
    };
    let login = state["login"].clone();
    match (method, parts.as_slice()) {
        ("GET", ["repos", _, _, "issues", number, "comments"]) => {
            let pr = find_pr(state, number)?;
            let comments: Vec<Value> = pr["comments"]
                .as_array()
                .map(|comments| {
                    comments
                        .iter()
                        .map(|comment| json!({"id": comment["id"], "body": comment["body"], "user": {"login": comment["login"]}}))
                        .collect()
                })
                .unwrap_or_default();
            Ok(Value::Array(comments).to_string())
        }
        ("POST", ["repos", _, _, "issues", number, "comments"]) => {
            let body = body.ok_or("fake gh: POST comment without a body")?;
            let id = state["next_comment_id"].as_u64().unwrap_or(1);
            state["next_comment_id"] = json!(id + 1);
            let pr = find_pr(state, number)?;
            let comment = json!({"id": id, "body": body, "login": login});
            if let Some(comments) = pr["comments"].as_array_mut() {
                comments.push(comment.clone());
            }
            Ok(comment.to_string())
        }
        ("PATCH", ["repos", _, _, "issues", "comments", id]) => {
            let body = body.ok_or("fake gh: PATCH comment without a body")?;
            let id: u64 = id.parse().map_err(|_| format!("fake gh: not a comment id: {id}"))?;
            for pr in prs(state)?.iter_mut() {
                if let Some(comments) = pr["comments"].as_array_mut() {
                    for comment in comments.iter_mut() {
                        if comment["id"] == id {
                            comment["body"] = json!(body);
                            return Ok(comment.to_string());
                        }
                    }
                }
            }
            Err(format!("HTTP 404: Not Found (comment {id})"))
        }
        _ => Err(format!("fake gh: unexpected api call: {method} {path}")),
    }
}

fn native(state: &Value) -> Result<(), String> {
    match state["native_stack"].as_str() {
        Some("linked" | "refused") => Ok(()),
        _ => Err("unknown command \"stack\" for \"gh\"".to_owned()),
    }
}

fn stack_link(state: &mut Value, rest: &[&str]) -> Result<String, String> {
    native(state)?;
    if state["native_stack"] == "refused" {
        return Err("stacked pull requests are not enabled for this repository".to_owned());
    }
    let mut urls = Vec::new();
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        if *word == "--base" || *word == "--remote" {
            words.next();
        } else if !word.starts_with("--") {
            urls.push(json!(word));
        }
    }
    if let Some(stacks) = state["stacks"].as_array_mut() {
        stacks.push(Value::Array(urls));
    }
    Ok(String::new())
}
