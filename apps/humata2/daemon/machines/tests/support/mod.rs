//! Fakes for driving setup without a machine: a runner that answers each
//! kind of command from a queue and records what it was asked to run, and
//! builds that hand out small fake files.

#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use humata2_machines::{
    Builds, BuildsError, Exit, InstalledFile, Invocation, Output, Payload, Platform, Purpose, RunError,
    Runner,
};

/// Answers each [`Purpose`] from its own queue, in order.
#[derive(Default)]
pub struct FakeRunner {
    answers: Mutex<HashMap<Purpose, VecDeque<Output>>>,
    seen: Mutex<Vec<Invocation>>,
}

impl FakeRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues what the next command for `purpose` prints and how it ends.
    pub fn answer(&self, purpose: Purpose, exit: Exit, stdout: &str, stderr: &str) -> &Self {
        self.answers
            .lock()
            .unwrap()
            .entry(purpose)
            .or_default()
            .push_back(Output {
                exit,
                stdout: stdout.as_bytes().to_vec(),
                stderr: stderr.as_bytes().to_vec(),
            });
        self
    }

    pub fn ok(&self, purpose: Purpose, stdout: &str) -> &Self {
        self.answer(purpose, Exit::Code(0), stdout, "")
    }

    /// Everything run so far, in order.
    pub fn seen(&self) -> Vec<Invocation> {
        self.seen.lock().unwrap().clone()
    }

    pub fn ran(&self, purpose: Purpose) -> Vec<Invocation> {
        self.seen()
            .into_iter()
            .filter(|invocation| invocation.purpose == purpose)
            .collect()
    }
}

impl Runner for FakeRunner {
    fn run(&self, invocation: &Invocation) -> Result<Output, RunError> {
        self.seen.lock().unwrap().push(invocation.clone());
        let answer = self
            .answers
            .lock()
            .unwrap()
            .get_mut(&invocation.purpose)
            .and_then(VecDeque::pop_front);
        Ok(answer.unwrap_or_else(|| panic!("the test gave no answer for {invocation:?}")))
    }
}

/// The same files for every platform, or none at all.
pub struct FakeBuilds(pub Option<Vec<Payload>>);

impl FakeBuilds {
    pub fn files() -> Self {
        Self(Some(payloads()))
    }

    pub fn none() -> Self {
        Self(None)
    }
}

impl Builds for FakeBuilds {
    fn payloads(&self, platform: Platform) -> Result<Vec<Payload>, BuildsError> {
        self.0.clone().ok_or_else(|| BuildsError::NoBuild {
            platform,
            shipped: String::new(),
        })
    }
}

/// A fake humatad2, holder and `humata`.
pub fn payloads() -> Vec<Payload> {
    vec![
        Payload::new(InstalledFile::Humatad, Arc::from(&b"#!/bin/sh\nexit 0\n"[..])),
        Payload::new(InstalledFile::Holder, Arc::from(&b"#!/bin/sh\nexit 1\n"[..])),
        Payload::new(InstalledFile::Cli, Arc::from(&b"#!/bin/sh\nexit 2\n"[..])),
    ]
}

/// What the check prints on a machine, one fact per field.
pub struct CheckFacts {
    pub os: &'static str,
    pub arch: &'static str,
    pub git: bool,
    pub disk_free_kib: u64,
    pub systemd: bool,
    pub cron: bool,
    pub installed: Vec<(InstalledFile, String)>,
    pub staged: Vec<(InstalledFile, String)>,
    pub holders: bool,
}

impl Default for CheckFacts {
    fn default() -> Self {
        Self {
            os: "Linux",
            arch: "x86_64",
            git: true,
            disk_free_kib: 50 * 1024 * 1024,
            systemd: true,
            cron: true,
            installed: Vec::new(),
            staged: Vec::new(),
            holders: false,
        }
    }
}

impl CheckFacts {
    pub fn stdout(&self) -> String {
        let yes = |value: bool| if value { "yes" } else { "no" };
        let mut out = format!(
            "Welcome to the test machine\nhumata2.os={}\nhumata2.arch={}\nhumata2.user=dev\nhumata2.home=/home/dev\nhumata2.git={}\nhumata2.disk_free_kib={}\nhumata2.systemd={}\nhumata2.linger=no\nhumata2.crontab={}\nhumata2.setsid={}\n",
            self.os,
            self.arch,
            yes(self.git),
            self.disk_free_kib,
            yes(self.systemd),
            yes(self.cron),
            yes(self.cron),
        );
        out.push_str(&format!("humata2.holders={}\n", yes(self.holders)));
        for (file, sha) in &self.staged {
            out.push_str(&format!("humata2.staged.{}={sha}\n", file.file_name()));
        }
        for file in InstalledFile::ALL {
            let sha = self
                .installed
                .iter()
                .find(|(installed, _)| *installed == file)
                .map_or("none", |(_, sha)| sha.as_str());
            out.push_str(&format!("humata2.sha.{}={sha}\n", file.file_name()));
        }
        out
    }
}
