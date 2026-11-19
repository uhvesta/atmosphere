//! A move's machine script triggers (D681): `session-leaving` on the old
//! machine before its agent stops, `session-arrived` on the new one once
//! the move is done. Scripts never fail or hold a move.

use std::time::Duration;

/// The longest a move waits for the old machine's leaving rules.
pub const LEAVING_WAIT: Duration = Duration::from_secs(60);

/// Lifecycle trigger fired on a machine during a session move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptTrigger {
    SessionLeaving,
    SessionArrived,
}

/// Session-specific metadata passed to machine scripts during a move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionScriptContext {
    pub session: String,
    pub channel: String,
    pub folder: String,
    pub from: String,
    pub to: String,
}

/// Full context for a machine script invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptContext {
    pub machine: String,
    pub trigger: ScriptTrigger,
    pub session: Option<SessionScriptContext>,
}

/// Builds the [`ScriptContext`] for a move's leaving or arrived trigger.
pub fn session_script_context(
    machine: &str,
    trigger: ScriptTrigger,
    session: &str,
    channel_name: &str,
    channel_folder: &str,
    session_folder: &str,
    from: &str,
    to: &str,
) -> ScriptContext {
    // The session's place under the machine's channels folder.
    let folder = if channel_folder.is_empty() {
        session_folder.to_owned()
    } else {
        format!("{channel_folder}/{session_folder}")
    };
    ScriptContext {
        machine: machine.to_owned(),
        trigger,
        session: Some(SessionScriptContext {
            session: session.to_owned(),
            channel: channel_name.to_owned(),
            folder,
            from: from.to_owned(),
            to: to.to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_context_joins_channel_and_session_folders() {
        let ctx = session_script_context(
            "m2",
            ScriptTrigger::SessionArrived,
            "se_1",
            "payments",
            "payments",
            "sessions/fix",
            "m1",
            "m2",
        );
        assert_eq!(ctx.machine, "m2");
        assert_eq!(ctx.trigger, ScriptTrigger::SessionArrived);
        let session = ctx.session.unwrap();
        assert_eq!(session.folder, "payments/sessions/fix");

        let desk = session_script_context(
            "m1",
            ScriptTrigger::SessionLeaving,
            "se_2",
            "Desk",
            "",
            "desk/scratch",
            "m1",
            "m2",
        );
        assert_eq!(desk.session.unwrap().folder, "desk/scratch");
    }
}
