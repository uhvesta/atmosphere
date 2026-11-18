//! The Scripts section of a machine's page (MACHINESCRIPTS, D686): the
//! owner's rules that apply to this machine, each with its last run, and
//! Run now. The state is what was last read from the host; nothing here
//! polls, the host's feed says when a run changes.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use humata2_clients::machines::{MachineScripts, ScriptRuleView, ScriptRun};

/// What the Scripts section shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptsState {
    /// What was last read; kept when a later read fails.
    pub loaded: Option<MachineScripts>,
    /// A read is in flight.
    pub loading: bool,
    /// Why the last read failed, as text.
    pub error: Option<String>,
    /// Rules whose Run now was sent and is awaited.
    pub running: BTreeSet<String>,
    /// Why a Run now was refused, by rule, as text.
    pub refused: BTreeMap<String, String>,
    /// The rule whose steps are open.
    pub open: Option<String>,
}

/// One rule as the section draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptRow<'a> {
    /// The rule.
    pub rule: &'a ScriptRuleView,
    /// Its last run on this machine.
    pub last: Option<&'a ScriptRun>,
    /// A run is going: Run now was sent, or the host says the last run is.
    pub running: bool,
    /// Why Run now was refused, when no run explains it.
    pub refused: Option<&'a str>,
    /// Its steps are open.
    pub open: bool,
}

impl ScriptsState {
    /// The rules, each with its last run, in the file's order.
    #[must_use]
    pub fn rows(&self) -> Vec<ScriptRow<'_>> {
        let Some(scripts) = &self.loaded else {
            return Vec::new();
        };
        scripts
            .rules
            .iter()
            .map(|rule| {
                let last = scripts.last_run(&rule.name);
                ScriptRow {
                    rule,
                    last,
                    running: self.running.contains(&rule.name)
                        || last.is_some_and(|run| run.outcome == humata2_clients::machines::ScriptOutcome::Running),
                    refused: self
                        .refused
                        .get(&rule.name)
                        .map(String::as_str)
                        .filter(|_| !last.is_some_and(|run| run.outcome.is_problem())),
                    open: self.open.as_deref() == Some(rule.name.as_str()),
                }
            })
            .collect()
    }

    /// Why the rules file can't be used, with what to change.
    #[must_use]
    pub fn problem(&self) -> Option<&str> {
        self.loaded.as_ref().and_then(|scripts| scripts.problem.as_deref())
    }

    /// Whether the section has anything to say. With no rules and no problem
    /// the machine page shows nothing for scripts.
    #[must_use]
    pub fn visible(&self) -> bool {
        self.problem().is_some()
            || self.error.is_some()
            || self.loaded.as_ref().is_some_and(|scripts| !scripts.rules.is_empty())
    }
}
