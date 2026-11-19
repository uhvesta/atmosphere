//! The run log: per machine, the last 50 runs as JSON lines under
//! `<state>/machine-scripts/<machine>.jsonl`, and which `once` rules have
//! passed on which machine in `once.json`.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use humata2_machines::Trigger;
use serde::{Deserialize, Serialize};

use super::{Outcome, RunRecord, StepRun};

/// Runs kept per machine.
const KEEP: usize = 50;

#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("{name:?} can't be a machine name here: a name with a '/' or '\\' in it, or an empty one, is refused. Use the machine's name as the machine list shows it")]
    BadMachine { name: String },
    #[error("can't read or write {path}: {source}. Check that the daemon's state folder is writable and has space, then try again")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{path} isn't the once record the daemon writes: {source}. Move it aside; the rules marked `once` then run again at their next trigger")]
    Once {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("can't write a run to the log: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Serialize, Deserialize)]
struct StoredRun {
    id: String,
    machine: String,
    rule: String,
    trigger: String,
    session: Option<String>,
    started_at_ms: i64,
    took_ms: u64,
    outcome: Outcome,
    steps: Vec<StoredStep>,
}

#[derive(Serialize, Deserialize)]
struct StoredStep {
    index: u32,
    kind: String,
    label: String,
    outcome: Outcome,
    exit: Option<i32>,
    tail: Vec<String>,
    error: Option<String>,
    took_ms: u64,
}

fn stored(run: &RunRecord) -> StoredRun {
    StoredRun {
        id: run.id.clone(),
        machine: run.machine.clone(),
        rule: run.rule.clone(),
        trigger: run.trigger.as_str().to_string(),
        session: run.session.clone(),
        started_at_ms: run.started_at_ms,
        took_ms: run.took_ms,
        outcome: run.outcome,
        steps: run
            .steps
            .iter()
            .map(|step| StoredStep {
                index: step.index,
                kind: step.kind.to_string(),
                label: step.label.clone(),
                outcome: step.outcome,
                exit: step.exit,
                tail: step.tail.clone(),
                error: step.error.clone(),
                took_ms: step.took_ms,
            })
            .collect(),
    }
}

fn kind_of(kind: &str) -> Option<&'static str> {
    ["run", "humata", "start", "send"].into_iter().find(|known| *known == kind)
}

/// `None` when the line names a trigger or step kind this build doesn't know.
fn restored(run: StoredRun) -> Option<RunRecord> {
    let trigger = Trigger::parse(&run.trigger)?;
    let mut steps = Vec::with_capacity(run.steps.len());
    for step in run.steps {
        steps.push(StepRun {
            index: step.index,
            kind: kind_of(&step.kind)?,
            label: step.label,
            outcome: step.outcome,
            exit: step.exit,
            tail: step.tail,
            error: step.error,
            took_ms: step.took_ms,
        });
    }
    Some(RunRecord {
        id: run.id,
        machine: run.machine,
        rule: run.rule,
        trigger,
        session: run.session,
        started_at_ms: run.started_at_ms,
        took_ms: run.took_ms,
        outcome: run.outcome,
        steps,
    })
}

pub(super) struct RunLog {
    dir: PathBuf,
    /// One writer at a time: appends and `once` updates read then rewrite.
    write: Mutex<()>,
}

impl RunLog {
    pub(super) fn new(state_dir: &Path) -> Self {
        RunLog {
            dir: state_dir.join("machine-scripts"),
            write: Mutex::new(()),
        }
    }

    fn machine_file(&self, machine: &str) -> Result<PathBuf, LogError> {
        if machine.is_empty() || machine.contains('/') || machine.contains('\\') || machine == "." || machine == ".." {
            return Err(LogError::BadMachine { name: machine.to_string() });
        }
        Ok(self.dir.join(format!("{machine}.jsonl")))
    }

    fn once_file(&self) -> PathBuf {
        self.dir.join("once.json")
    }

    /// Adds `run` to its machine's log, keeping the last 50.
    pub(super) fn append(&self, run: &RunRecord) -> Result<(), LogError> {
        let path = self.machine_file(&run.machine)?;
        let line = serde_json::to_string(&stored(run)).map_err(|source| LogError::Encode { source })?;
        let _write = self.write.lock().unwrap_or_else(PoisonError::into_inner);
        let mut lines: Vec<String> = read_text(&path)?
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(str::to_string)
            .collect();
        lines.push(line);
        if lines.len() > KEEP {
            lines.drain(..lines.len() - KEEP);
        }
        let mut text = lines.join("\n");
        text.push('\n');
        write_atomic(&path, text.as_bytes())
    }

    /// The machine's runs, newest first, of `rule` if given, at most `limit`.
    /// A line this build can't read is left out and logged.
    pub(super) fn read(&self, machine: &str, rule: Option<&str>, limit: usize) -> Result<Vec<RunRecord>, LogError> {
        let path = self.machine_file(machine)?;
        let text = read_text(&path)?;
        let mut runs = Vec::new();
        for line in text.lines().rev().filter(|line| !line.trim().is_empty()) {
            if runs.len() >= limit {
                break;
            }
            let run = serde_json::from_str::<StoredRun>(line).ok().and_then(restored);
            let Some(run) = run else {
                humata2_rpc_kit::log::line(&format!(
                    "humatad2: a line of the machine script log {} isn't readable and is left out; it goes when newer runs push it out",
                    path.display()
                ));
                continue;
            };
            if rule.is_none_or(|wanted| run.rule == wanted) {
                runs.push(run);
            }
        }
        Ok(runs)
    }

    fn once_passes(&self) -> Result<BTreeMap<String, Vec<String>>, LogError> {
        let path = self.once_file();
        let text = read_text(&path)?;
        if text.trim().is_empty() {
            return Ok(BTreeMap::new());
        }
        serde_json::from_str(&text).map_err(|source| LogError::Once { path, source })
    }

    /// Whether `rule` has passed on `machine` before.
    pub(super) fn once_passed(&self, machine: &str, rule: &str) -> Result<bool, LogError> {
        self.machine_file(machine)?;
        let _write = self.write.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(self.once_passes()?.get(machine).is_some_and(|rules| rules.iter().any(|name| name == rule)))
    }

    /// Records that `rule` passed on `machine`.
    pub(super) fn mark_once(&self, machine: &str, rule: &str) -> Result<(), LogError> {
        self.machine_file(machine)?;
        let _write = self.write.lock().unwrap_or_else(PoisonError::into_inner);
        let mut passes = self.once_passes()?;
        let rules = passes.entry(machine.to_string()).or_default();
        if rules.iter().any(|name| name == rule) {
            return Ok(());
        }
        rules.push(rule.to_string());
        let text = serde_json::to_string_pretty(&passes).map_err(|source| LogError::Encode { source })?;
        write_atomic(&self.once_file(), text.as_bytes())
    }
}

fn read_text(path: &Path) -> Result<String, LogError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(String::new()),
        Err(source) => Err(LogError::Io { path: path.to_path_buf(), source }),
    }
}

/// Writes next to `path` and renames over it, so a cut never leaves half a file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), LogError> {
    let io = |source| LogError::Io { path: path.to_path_buf(), source };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io)?;
    }
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    fs::write(&temp, bytes).map_err(io)?;
    fs::rename(&temp, path).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(machine: &str, rule: &str, id: &str) -> RunRecord {
        RunRecord {
            id: id.to_string(),
            machine: machine.to_string(),
            rule: rule.to_string(),
            trigger: Trigger::Connected,
            session: None,
            started_at_ms: 1,
            took_ms: 2,
            outcome: Outcome::Failed,
            steps: vec![StepRun {
                index: 0,
                kind: "run",
                label: "a.sh".to_string(),
                outcome: Outcome::Failed,
                exit: Some(3),
                tail: vec!["boom".to_string()],
                error: Some("exit 3".to_string()),
                took_ms: 2,
            }],
        }
    }

    #[test]
    fn a_run_reads_back_as_written() {
        let dir = tempfile::tempdir().unwrap();
        let log = RunLog::new(dir.path());
        log.append(&run("box", "r", "1")).unwrap();
        let read = log.read("box", None, 10).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].id, "1");
        assert_eq!(read[0].outcome, Outcome::Failed);
        assert_eq!(read[0].steps[0].kind, "run");
        assert_eq!(read[0].steps[0].exit, Some(3));
        assert_eq!(read[0].steps[0].tail, vec!["boom".to_string()]);
    }

    #[test]
    fn the_last_fifty_are_kept_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let log = RunLog::new(dir.path());
        for n in 0..55 {
            log.append(&run("box", "r", &n.to_string())).unwrap();
        }
        let read = log.read("box", None, 100).unwrap();
        assert_eq!(read.len(), 50);
        assert_eq!(read[0].id, "54");
        assert_eq!(read[49].id, "5");
        assert_eq!(log.read("box", None, 3).unwrap().len(), 3);
    }

    #[test]
    fn reads_filter_by_rule_and_keep_machines_apart() {
        let dir = tempfile::tempdir().unwrap();
        let log = RunLog::new(dir.path());
        log.append(&run("box", "a", "1")).unwrap();
        log.append(&run("box", "b", "2")).unwrap();
        log.append(&run("other", "a", "3")).unwrap();
        let only_a = log.read("box", Some("a"), 10).unwrap();
        assert_eq!(only_a.len(), 1);
        assert_eq!(only_a[0].id, "1");
        assert_eq!(log.read("other", None, 10).unwrap().len(), 1);
        assert!(log.read("nobody", None, 10).unwrap().is_empty());
    }

    #[test]
    fn a_name_with_a_slash_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let log = RunLog::new(dir.path());
        assert!(matches!(log.read("a/b", None, 1), Err(LogError::BadMachine { .. })));
        assert!(matches!(log.append(&run("../x", "r", "1")), Err(LogError::BadMachine { .. })));
        assert!(matches!(log.mark_once("a/b", "r"), Err(LogError::BadMachine { .. })));
    }

    #[test]
    fn once_passes_are_per_machine_and_rule() {
        let dir = tempfile::tempdir().unwrap();
        let log = RunLog::new(dir.path());
        assert!(!log.once_passed("box", "r").unwrap());
        log.mark_once("box", "r").unwrap();
        log.mark_once("box", "r").unwrap();
        assert!(log.once_passed("box", "r").unwrap());
        assert!(!log.once_passed("box", "s").unwrap());
        assert!(!log.once_passed("other", "r").unwrap());
    }

    #[test]
    fn a_broken_once_file_is_an_error_not_a_pass() {
        let dir = tempfile::tempdir().unwrap();
        let log = RunLog::new(dir.path());
        fs::create_dir_all(dir.path().join("machine-scripts")).unwrap();
        fs::write(dir.path().join("machine-scripts/once.json"), "{nope").unwrap();
        assert!(matches!(log.once_passed("box", "r"), Err(LogError::Once { .. })));
    }
}
