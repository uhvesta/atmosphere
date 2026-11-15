//! Fake daemon services for replaying a recording: each call is answered
//! as the daemon answered it when the report was taken, so a report can
//! become a regression test with no daemon running.

use crate::error::FakeDaemonError;
use crate::event::{DaemonError, Event, Rpc};
use crate::recording::Captured;

/// The daemon calls and errors of a recording, to answer a replay's calls
/// with. The app's fake services look each call up here and answer with
/// its status and response.
///
/// Answers are the saved, scrubbed ones: a seeded secret reads as its
/// `‹NAME›` marker, and terminal output or paste-folder contents that were
/// left out read as a marker too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FakeDaemon {
    calls: Vec<Rpc>,
    answered: Vec<bool>,
    errors: Vec<DaemonError>,
}

impl Captured {
    /// Fake services that answer as the daemon did during this recording.
    /// A snapshot's answer nothing.
    pub fn fake_daemon(&self) -> FakeDaemon {
        let mut calls = Vec::new();
        let mut errors = Vec::new();
        for event in &self.events {
            match event {
                Event::Rpc(rpc) => calls.push(rpc.clone()),
                Event::DaemonError(error) => errors.push(error.clone()),
                _ => {}
            }
        }
        FakeDaemon {
            answered: vec![false; calls.len()],
            calls,
            errors,
        }
    }
}

impl FakeDaemon {
    /// The recorded answer to the next call of `method` on `machine`: the
    /// first such call in the recording not answered yet. Calls are
    /// matched by machine and method, in order, and not by request, since
    /// a request can hold ids or times made fresh in each run; compare
    /// [`Rpc::request`] yourself where it matters.
    ///
    /// # Errors
    ///
    /// [`FakeDaemonError::Unrecorded`] when the recording has no such call
    /// left.
    pub fn answer(&mut self, machine: &str, method: &str) -> Result<&Rpc, FakeDaemonError> {
        let matches = |call: &Rpc| call.machine == machine && call.method == method;
        let next = self
            .calls
            .iter()
            .zip(&self.answered)
            .position(|(call, answered)| !answered && matches(call));
        match next {
            Some(index) => {
                self.answered[index] = true;
                Ok(&self.calls[index])
            }
            None => Err(FakeDaemonError::Unrecorded {
                machine: machine.to_owned(),
                method: method.to_owned(),
                recorded: self.calls.iter().filter(|call| matches(call)).count(),
            }),
        }
    }

    /// The recorded calls the replay hasn't made, in order. A replay that
    /// leaves some is no longer doing what the recording did.
    pub fn unanswered(&self) -> Vec<&Rpc> {
        self.calls
            .iter()
            .zip(&self.answered)
            .filter(|(_, answered)| !**answered)
            .map(|(call, _)| call)
            .collect()
    }

    /// The errors daemons reported outside a call (a stream that broke, a
    /// holder that died), in order, for the fake services to raise at the
    /// same point.
    pub fn errors(&self) -> &[DaemonError] {
        &self.errors
    }
}
