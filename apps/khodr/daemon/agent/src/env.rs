//! Variables for the programs agents start.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

/// Variables set for the programs agents start: the Copilot CLI and the
/// commands a session runs. A host gets them from the daemon after it
/// connects, so a token reaches the agents without being written to a file
/// on the host. Clones share the same variables.
#[derive(Clone, Debug, Default)]
pub struct Env(Arc<RwLock<BTreeMap<String, String>>>);

impl Env {
    pub fn new(vars: impl IntoIterator<Item = (String, String)>) -> Self {
        Self(Arc::new(RwLock::new(vars.into_iter().collect())))
    }

    /// Replaces every variable; programs started from now on see the new ones.
    pub fn replace(&self, vars: BTreeMap<String, String>) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = vars;
    }

    pub fn vars(&self) -> Vec<(String, String)> {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    }

    /// Whether `name` is set to something.
    pub fn has(&self, name: &str) -> bool {
        self.0
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
}
