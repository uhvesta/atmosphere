//! `CliAuthService` over gRPC (`humata login` pending approvals, CLI sessions, script tokens).

use humata2_clients::daemon::{CliSession, PendingCliLogin, ScriptToken};
use humata2_clients::model::{Capability, ChannelId, MachineId};
use humata2_clients::DaemonError;

use crate::connection::{Shared, not_in_this_release};
use crate::humata2_grpc::humata::v1 as pb;
use crate::humata2_grpc::Timestamp;

fn timestamp_ms(ts: Option<&Timestamp>) -> i64 {
    let Some(ts) = ts else {
        return 0;
    };
    ts.seconds
        .saturating_mul(1_000)
        .saturating_add(i64::from(ts.nanos / 1_000_000))
}

pub(crate) fn pending_login(wire: &pb::PendingCliLogin) -> PendingCliLogin {
    PendingCliLogin {
        id: wire.id.clone(),
        terminal: wire.terminal.clone(),
        hours: wire.hours,
        asked_at_unix_ms: timestamp_ms(wire.asked_at.as_ref()),
    }
}

fn cli_session(wire: &pb::CliSession, fallback_machine: &MachineId) -> CliSession {
    let machine = wire
        .machine
        .as_ref()
        .filter(|id| !id.value.is_empty())
        .map_or_else(|| fallback_machine.clone(), |id| MachineId::new(id.value.clone()));
    CliSession {
        id: wire.id.clone(),
        machine,
        terminal: wire.terminal.clone(),
        expires_at_unix_ms: timestamp_ms(wire.expires_at.as_ref()),
    }
}

fn script_token(wire: &pb::list_script_tokens_response::Entry) -> ScriptToken {
    ScriptToken {
        id: wire.token_id.clone(),
        name: wire.name.clone(),
        capabilities: wire
            .capabilities
            .iter()
            .filter_map(|&value| pb::Capability::try_from(value).ok())
            .map(|cap| Capability::new(cap.as_str_name().to_ascii_lowercase()))
            .collect(),
        channels: wire
            .channels
            .iter()
            .map(|id| ChannelId::new(id.value.clone()))
            .collect(),
        expires_at_unix_ms: timestamp_ms(wire.expires_at.as_ref()),
        last_used_at_unix_ms: wire.last_used_at.as_ref().map(|ts| timestamp_ms(Some(ts))),
    }
}

impl Shared {
    pub(crate) async fn list_pending_cli_logins(&self) -> Result<Vec<PendingCliLogin>, DaemonError> {
        let reply = match self
            .cli_auth()
            .list_pending_cli_logins(pb::ListPendingCliLoginsRequest {})
            .await
        {
            Ok(reply) => reply.into_inner(),
            Err(status) if status.code() == tonic::Code::Unimplemented => return Ok(Vec::new()),
            Err(status) => return Err(self.error(&status, "Listing pending CLI logins")),
        };
        Ok(reply.logins.iter().map(pending_login).collect())
    }

    pub(crate) async fn answer_cli_login(&self, id: String, allow: bool) -> Result<(), DaemonError> {
        let what = if allow {
            "Approving the CLI login"
        } else {
            "Denying the CLI login"
        };
        self.cli_auth()
            .answer_cli_login(pb::AnswerCliLoginRequest { id, allow })
            .await
            .map_err(|status| {
                if status.code() == tonic::Code::Unimplemented {
                    not_in_this_release(what)
                } else {
                    self.error(&status, what)
                }
            })?;
        Ok(())
    }

    pub(crate) async fn list_cli_sessions(&self) -> Result<Vec<CliSession>, DaemonError> {
        let reply = match self
            .cli_auth()
            .list_cli_sessions(pb::ListCliSessionsRequest {})
            .await
        {
            Ok(reply) => reply.into_inner(),
            Err(status) if status.code() == tonic::Code::Unimplemented => return Ok(Vec::new()),
            Err(status) => return Err(self.error(&status, "Listing CLI sessions")),
        };
        Ok(reply
            .sessions
            .iter()
            .map(|session| cli_session(session, &self.machine))
            .collect())
    }

    pub(crate) async fn end_cli_session(&self, id: String) -> Result<(), DaemonError> {
        const WHAT: &str = "Ending the CLI session";
        self.cli_auth()
            .end_cli_session(pb::EndCliSessionRequest { id })
            .await
            .map_err(|status| {
                if status.code() == tonic::Code::Unimplemented {
                    not_in_this_release(WHAT)
                } else {
                    self.error(&status, WHAT)
                }
            })?;
        Ok(())
    }

    pub(crate) async fn list_script_tokens(&self) -> Result<Vec<ScriptToken>, DaemonError> {
        let reply = match self
            .cli_auth()
            .list_script_tokens(pb::ListScriptTokensRequest {})
            .await
        {
            Ok(reply) => reply.into_inner(),
            Err(status) if status.code() == tonic::Code::Unimplemented => return Ok(Vec::new()),
            Err(status) => return Err(self.error(&status, "Listing script tokens")),
        };
        Ok(reply.tokens.iter().map(script_token).collect())
    }

    pub(crate) async fn revoke_script_token(&self, token_id: String) -> Result<(), DaemonError> {
        const WHAT: &str = "Revoking the script token";
        self.cli_auth()
            .revoke_script_token(pb::RevokeScriptTokenRequest {
                token_id,
                presence: None,
            })
            .await
            .map_err(|status| {
                if status.code() == tonic::Code::Unimplemented {
                    not_in_this_release(WHAT)
                } else {
                    self.error(&status, WHAT)
                }
            })?;
        Ok(())
    }
}
