//! A scratch world for one test: a bare git remote with a `main` branch, a
//! clone to commit in, and the fake gh on PATH with its state file.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

pub struct World {
    _root: tempfile::TempDir,
    pub remote: PathBuf,
    pub work: PathBuf,
    bin: PathBuf,
    state: PathBuf,
    home: PathBuf,
    /// `$HUMATA2_HOME` for every stack run: stack.toml goes in its `config/`.
    pub humata_home: PathBuf,
}

/// What a run of `stack` printed and how it ended.
pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    pub fn ok(&self) -> &Run {
        assert_eq!(self.code, 0, "stack failed\nstdout:\n{}\nstderr:\n{}", self.stdout, self.stderr);
        self
    }
}

fn env_path(name: &str) -> PathBuf {
    let path = std::env::var_os(name).unwrap_or_else(|| panic!("{name} is not set; run through bazel test"));
    std::fs::canonicalize(&path).unwrap_or_else(|error| panic!("{name}={path:?}: {error}"))
}

impl World {
    pub fn new() -> World {
        let root = tempfile::tempdir().expect("temp dir");
        let base = root.path().canonicalize().expect("canonical temp dir");
        let remote = base.join("remote.git");
        let work = base.join("work");
        let bin = base.join("bin");
        let home = base.join("home");
        for dir in [&bin, &home] {
            std::fs::create_dir_all(dir).expect("mkdir");
        }
        std::os::unix::fs::symlink(env_path("FAKE_GH"), bin.join("gh")).expect("link fake gh");
        let world = World {
            _root: root,
            remote,
            work,
            bin,
            state: base.join("gh-state.json"),
            home,
            humata_home: base.join("humata-home"),
        };
        world.write_state(&json!({
            "login": "alice",
            "repo": "o/r",
            "default_branch": "main",
            "native_stack": "absent",
            "next_number": 1,
            "next_comment_id": 1,
            "prs": [],
            "stacks": [],
            "calls": [],
        }));
        world.git_in(&base, &["init", "--quiet", "--bare", "--initial-branch=main", "remote.git"]);
        world.git_in(&base, &["clone", "--quiet", "remote.git", "work"]);
        world.write("README.md", "hello\n");
        world.git(&["add", "README.md"]);
        world.git(&["commit", "--quiet", "-m", "first"]);
        world.git(&["push", "--quiet", "origin", "HEAD:refs/heads/main"]);
        world.git(&["remote", "set-head", "origin", "main"]);
        world
    }

    fn command(&self, program: &Path, dir: &Path) -> Command {
        let mut command = Command::new(program);
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![self.bin.clone()];
        paths.extend(std::env::split_paths(&path));
        command
            .current_dir(dir)
            .env("PATH", std::env::join_paths(paths).expect("PATH"))
            .env("HOME", &self.home)
            .env("HUMATA2_HOME", &self.humata_home)
            .env("FAKE_GH_STATE", &self.state)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.home.join("gitconfig"))
            .env("GIT_AUTHOR_NAME", "Alice")
            .env("GIT_AUTHOR_EMAIL", "alice@example.com")
            .env("GIT_COMMITTER_NAME", "Alice")
            .env("GIT_COMMITTER_EMAIL", "alice@example.com")
            .env_remove("BUILD_WORKING_DIRECTORY")
            .env_remove("GH_TOKEN")
            .env_remove("GITHUB_TOKEN");
        command
    }

    fn git_in(&self, dir: &Path, args: &[&str]) -> String {
        let output = self.command(Path::new("git"), dir).args(args).output().expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim_end().to_owned()
    }

    /// git in the clone.
    pub fn git(&self, args: &[&str]) -> String {
        self.git_in(&self.work, args)
    }

    /// git in the bare remote.
    pub fn remote_git(&self, args: &[&str]) -> String {
        self.git_in(&self.remote, args)
    }

    pub fn write(&self, file: &str, text: &str) {
        let path = self.work.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, text).expect("write file");
    }

    /// Writes `file` and commits it with `message`; the new sha.
    pub fn commit(&self, file: &str, text: &str, message: &str) -> String {
        self.write(file, text);
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    /// Writes `file` and amends HEAD with it, keeping the message.
    pub fn amend(&self, file: &str, text: &str) -> String {
        self.write(file, text);
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "--amend", "--no-edit"]);
        self.git(&["rev-parse", "HEAD"])
    }

    /// The sha of a branch on the remote, or None.
    pub fn remote_branch(&self, branch: &str) -> Option<String> {
        let output = self
            .command(Path::new("git"), &self.remote)
            .args(["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")])
            .output()
            .expect("run git");
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    /// Runs the built stack in the clone.
    pub fn stack(&self, args: &[&str]) -> Run {
        self.stack_from(&env_path("STACK_BIN"), &self.humata_home, args)
    }

    /// The stack binary under test, for copying elsewhere.
    pub fn stack_bin(&self) -> PathBuf {
        env_path("STACK_BIN")
    }

    /// Runs the `stack` binary at `program` with `$HUMATA2_HOME` set to
    /// `humata_home`.
    pub fn stack_from(&self, program: &Path, humata_home: &Path, args: &[&str]) -> Run {
        let output: Output = self
            .command(program, &self.work)
            .env("HUMATA2_HOME", humata_home)
            .args(args)
            .output()
            .expect("run stack");
        Run {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    /// Writes `name` (such as `stack.toml`) into `$HUMATA2_HOME/config/`.
    pub fn config(&self, name: &str, text: &str) {
        let dir = self.humata_home.join("config");
        std::fs::create_dir_all(&dir).expect("mkdir config");
        std::fs::write(dir.join(name), text).expect("write config file");
    }

    pub fn state(&self) -> Value {
        let text = std::fs::read_to_string(&self.state).expect("read fake gh state");
        serde_json::from_str(&text).expect("parse fake gh state")
    }

    pub fn write_state(&self, state: &Value) {
        std::fs::write(&self.state, serde_json::to_string_pretty(state).expect("state json")).expect("write state");
    }

    /// Changes the fake GitHub's state.
    pub fn edit_state(&self, change: impl FnOnce(&mut Value)) {
        let mut state = self.state();
        change(&mut state);
        self.write_state(&state);
    }

    /// The fake's pull requests.
    pub fn prs(&self) -> Vec<Value> {
        self.state()["prs"].as_array().cloned().unwrap_or_default()
    }

    /// The pull request whose head is `branch`.
    pub fn pr(&self, branch: &str) -> Value {
        self.prs()
            .into_iter()
            .find(|pr| pr["head"] == branch)
            .unwrap_or_else(|| panic!("no PR for {branch} in {:#?}", self.prs()))
    }

    /// The gh calls made so far, each joined with spaces.
    pub fn calls(&self) -> Vec<String> {
        self.state()["calls"]
            .as_array()
            .map(|calls| {
                calls
                    .iter()
                    .map(|call| {
                        call.as_array()
                            .map(|words| words.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "))
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The history table rows (lines starting `| <digit>`) of a PR's comments.
pub fn history_rows(pr: &Value) -> Vec<String> {
    let comments: Vec<&Value> = pr["comments"]
        .as_array()
        .map(|comments| comments.iter().filter(|c| c["body"].as_str().is_some_and(|b| b.contains("<!-- stack:history"))).collect())
        .unwrap_or_default();
    assert!(comments.len() <= 1, "more than one history comment: {comments:#?}");
    comments
        .first()
        .and_then(|comment| comment["body"].as_str())
        .map(|body| {
            body.lines()
                .filter(|line| line.starts_with("| ") && line[2..].starts_with(|c: char| c.is_ascii_digit()))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// A commit message with a body in the template's sections and trailers.
pub fn message(subject: &str, trailers: &str) -> String {
    format!(
        "{subject}\n\n## Summary\nWhat {subject} changes.\n\n## Why\nBecause.\n\n## Testing\nbazel test //...\n\n{trailers}\n"
    )
}
