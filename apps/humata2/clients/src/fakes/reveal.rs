//! The Finder fake the tests and the fake app run on.

use std::sync::{Arc, Mutex};

use crate::{Reveal, RevealError};

/// A Finder that remembers what it was asked to show.
#[derive(Clone, Debug, Default)]
pub struct FakeReveal {
    revealed: Arc<Mutex<Vec<String>>>,
    edited: Arc<Mutex<Vec<String>>>,
    folders: Arc<Mutex<Vec<String>>>,
}

impl FakeReveal {
    /// The paths revealed so far, oldest first.
    #[must_use]
    pub fn revealed(&self) -> Vec<String> {
        match self.revealed.lock() {
            Ok(revealed) => revealed.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl Reveal for FakeReveal {
    fn reveal(&self, path: &str) -> Result<(), RevealError> {
        let mut revealed = match self.revealed.lock() {
            Ok(revealed) => revealed,
            Err(poisoned) => poisoned.into_inner(),
        };
        revealed.push(path.to_owned());
        Ok(())
    }

    fn edit(&self, path: &str, _starter: &str) -> Result<(), RevealError> {
        push_path(&self.edited, path);
        Ok(())
    }

    fn reveal_folder(&self, path: &str) -> Result<(), RevealError> {
        push_path(&self.folders, path);
        Ok(())
    }
}

fn push_path(list: &Mutex<Vec<String>>, path: &str) {
    let mut list = match list.lock() {
        Ok(list) => list,
        Err(poisoned) => poisoned.into_inner(),
    };
    list.push(path.to_owned());
}

fn paths(list: &Mutex<Vec<String>>) -> Vec<String> {
    match list.lock() {
        Ok(list) => list.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

impl FakeReveal {
    /// The files opened in an editor so far, oldest first.
    #[must_use]
    pub fn edited(&self) -> Vec<String> {
        paths(&self.edited)
    }

    /// The folders shown so far, oldest first.
    #[must_use]
    pub fn folders(&self) -> Vec<String> {
        paths(&self.folders)
    }
}
