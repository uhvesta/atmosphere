//! Variables for the programs agents start.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, PoisonError, RwLock};

/// Variables set for the programs agents start: the Copilot CLI and the
/// commands a session runs. A host gets them from the daemon after it
/// connects, so a token reaches the agents without being written to a file
/// on the host. Clones share the same variables.
#[derive(Clone, Debug, Default)]
pub struct Env {
    vars: Arc<RwLock<BTreeMap<String, String>>>,
    /// The names of the variables the host's model providers take their
    /// keys from: a session on no provider is never given them.
    provider_variables: Arc<RwLock<BTreeSet<String>>>,
}

impl Env {
    pub fn new(vars: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            vars: Arc::new(RwLock::new(vars.into_iter().collect())),
            provider_variables: Arc::default(),
        }
    }

    /// Replaces every variable; programs started from now on see the new ones.
    pub fn replace(&self, vars: BTreeMap<String, String>) {
        *self.vars.write().unwrap_or_else(PoisonError::into_inner) = vars;
    }

    /// Replaces the names of the variables the host's providers take their
    /// keys from.
    pub fn set_provider_variables(&self, names: impl IntoIterator<Item = String>) {
        *self
            .provider_variables
            .write()
            .unwrap_or_else(PoisonError::into_inner) = names.into_iter().collect();
    }

    /// The names of the variables the host's providers take their keys
    /// from.
    pub fn provider_variables(&self) -> BTreeSet<String> {
        self.provider_variables
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn vars(&self) -> Vec<(String, String)> {
        self.vars
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    }

    /// The value of `name`, if set.
    pub fn get(&self, name: &str) -> Option<String> {
        self.vars
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
    }

    /// Whether `name` is set to something.
    pub fn has(&self, name: &str) -> bool {
        self.vars
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .is_some_and(|value| !value.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clone_sees_what_replaces_the_variables() {
        let env = Env::new([("A".to_owned(), "1".to_owned())]);
        let shared = env.clone();
        env.replace(BTreeMap::from([(
            "GH_TOKEN".to_owned(),
            "secret".to_owned(),
        )]));
        assert_eq!(
            shared.vars(),
            [("GH_TOKEN".to_owned(), "secret".to_owned())]
        );
        assert!(shared.has("GH_TOKEN"));
        assert!(!shared.has("A"));
    }

    #[test]
    fn a_clone_sees_the_provider_variables_set() {
        let env = Env::default();
        let shared = env.clone();
        env.set_provider_variables(["ANTHROPIC_API_KEY".to_owned()]);
        assert_eq!(
            shared.provider_variables(),
            BTreeSet::from(["ANTHROPIC_API_KEY".to_owned()])
        );
        env.set_provider_variables([]);
        assert!(shared.provider_variables().is_empty());
    }
}
