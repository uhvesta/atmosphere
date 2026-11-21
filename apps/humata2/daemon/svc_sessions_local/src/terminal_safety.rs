//! Hard terminal-typing safety check for local sessions.
//!
//! Refuses typing into a target session's terminal when that session is
//! waiting on a permission prompt so only the owner may answer it.

pub use humata2_access::{
    PERMISSION_PROMPT_EXIT_CODE, PERMISSION_PROMPT_PROTECTED_CODE, TerminalTypeRefusal,
};
use humata2_rpc_kit::failure;
use humata2_svc_sessions_common::pb;
use tonic::{Code, Status};

/// Enforces the hard safety rule before typing into a target session's
/// terminal.
///
/// When `target_awaiting_permission_prompt` is true, returns
/// `Code::PermissionDenied` with a [`pb::Error`] containing the
/// [`TerminalTypeRefusal`] message (exit 4) and
/// [`PERMISSION_PROMPT_PROTECTED_CODE`].
pub fn enforce_terminal_type_safety(
    target_awaiting_permission_prompt: bool,
    target_title: &str,
) -> Result<(), Status> {
    humata2_access::check_terminal_type_safety(target_awaiting_permission_prompt, target_title)
        .map_err(|err| {
            failure(
                Code::PermissionDenied,
                pb::Error {
                    message: format!("{err} [{PERMISSION_PROMPT_PROTECTED_CODE}]"),
                    fix: "Wait for the owner to answer the permission prompt in the target session before typing into its terminal.".to_owned(),
                    retry: None,
                    kind: None,
                },
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_typing_when_not_awaiting_permission_prompt() {
        assert!(enforce_terminal_type_safety(false, "Restart doctor").is_ok());
    }

    #[test]
    fn refuses_typing_when_awaiting_permission_prompt() {
        assert_eq!(PERMISSION_PROMPT_EXIT_CODE, 4);
        assert_eq!(
            PERMISSION_PROMPT_PROTECTED_CODE,
            "permission_prompt_protected"
        );

        let err = enforce_terminal_type_safety(true, "Restart doctor").unwrap_err();
        assert_eq!(err.code(), Code::PermissionDenied);
        let msg = err.message();
        assert!(msg.contains("exit 4"), "unexpected message: {msg}");
        assert!(
            msg.contains("permission_prompt_protected"),
            "unexpected message: {msg}"
        );
        assert!(msg.contains("Restart doctor"), "unexpected message: {msg}");
    }
}
