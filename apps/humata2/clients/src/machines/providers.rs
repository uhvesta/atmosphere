//! Machine providers as Settings › Machines › Providers lists them
//! (PROVIDERS): what each one runs, the host variables it reads and the
//! machines it made, and a pasted provider file under review before import.
//!
//! Host variable values never appear here: only whether each one is set.

use super::ProviderParam;

/// One command a provider runs, as the providers file writes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderCommand {
    /// What it does: `create`, `connect`, `start`, `stop`, `status`, `delete`.
    pub action: String,
    /// The command, verbatim.
    pub command: String,
}

/// A host variable a provider's commands read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderVariable {
    /// Its name.
    pub name: String,
    /// Whether a value is kept for it. The value itself never comes back.
    pub set: bool,
}

/// A provider, for the Providers block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderEntry {
    /// Its id in the providers file.
    pub id: String,
    /// What to call it.
    pub label: String,
    /// Whether Humata ships it (MP-22), so it can't be removed.
    pub built_in: bool,
    /// The actions it defines, in the order the providers file names them.
    pub actions: Vec<String>,
    /// Every command it runs.
    pub commands: Vec<ProviderCommand>,
    /// What it asks for when a machine is added, with the defaults.
    pub params: Vec<ProviderParam>,
    /// The host variables its commands read.
    pub variables: Vec<ProviderVariable>,
    /// The names of the machines it made that Humata still has.
    pub machines: Vec<String>,
}

impl ProviderEntry {
    /// The variables its commands read that have no value yet.
    pub fn unset(&self) -> impl Iterator<Item = &str> {
        self.variables.iter().filter(|variable| !variable.set).map(|variable| variable.name.as_str())
    }
}

/// Something wrong in a pasted provider file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderProblem {
    /// The provider it is in, when the file got that far.
    pub provider: String,
    /// The field, when one is to blame.
    pub field: String,
    /// The line, from 1; 0 when no line is to blame.
    pub line: u32,
    /// What is wrong and how to fix it.
    pub message: String,
}

impl std::fmt::Display for ProviderProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line > 0 {
            write!(f, "Line {}: ", self.line)?;
        }
        if !self.field.is_empty() {
            write!(f, "{}: ", self.field)?;
        }
        f.write_str(&self.message)
    }
}

/// What importing a pasted provider file would add (MP-23). Nothing is added
/// until [`super::Machines::import_provider`] is called with it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProviderImportPlan {
    /// The providers in the file, each with every command it will run and
    /// the variables it reads.
    pub providers: Vec<ProviderEntry>,
    /// The ids of the providers it replaces, because they exist already.
    pub replaces: Vec<String>,
    /// Why the file can't be imported; empty when it can.
    pub problems: Vec<ProviderProblem>,
    /// What imports exactly this plan, once; `None` when there are problems.
    pub token: Option<String>,
}
