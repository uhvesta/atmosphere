//! The keychain fake the tests and the fake app run on.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::{Keychain, KeychainError};

/// A keychain in memory.
#[derive(Clone, Debug, Default)]
pub struct FakeKeychain {
    values: Arc<Mutex<HashMap<String, String>>>,
}

impl FakeKeychain {
    fn values(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        match self.values.lock() {
            Ok(values) => values,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Keychain for FakeKeychain {
    fn has(&self, name: &str) -> Result<bool, KeychainError> {
        Ok(self.values().contains_key(name))
    }

    fn set(&self, name: &str, value: &str) -> Result<(), KeychainError> {
        self.values().insert(name.to_owned(), value.to_owned());
        Ok(())
    }
}
