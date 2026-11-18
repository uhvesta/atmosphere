//! `humata login` and `humata logout` (CLI.md §5.4, SEC-11): a login is
//! confirmed in the app, and its token kept for this terminal session only.

use humata2_proto::humata::v1 as pb;
use prost::Message;
use serde_json::json;

use crate::as_json;
use crate::call::{Ctx, Done, decode};
use crate::console::Console;
use crate::daemon::{Daemon, DaemonError, rpc};
use crate::error::CliError;
use crate::login;
use crate::principal::Principal;

/// gRPC's NOT_FOUND and UNAUTHENTICATED: what ending a login the daemon no
/// longer has answers.
const NOT_FOUND: i32 = 5;
const UNAUTHENTICATED: i32 = 16;

/// Asks the daemon for a login, which the person confirms in the app, and
/// saves its token for this terminal session. The token is never printed.
pub(super) async fn login<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    hours: u32,
) -> Result<Done, CliError> {
    let path = match login::path(ctx.environment) {
        Some(path) => path,
        None if ctx.environment.home.is_none() => return Err(CliError::LoginNeedsHome),
        None => return Err(CliError::LoginNeedsTerminal),
    };
    let rpc = rpc::cli_auth::BEGIN_CLI_SESSION;
    if ctx.flags.dry_run {
        return ctx
            .change_present(rpc, |presence| pb::BeginCliSessionRequest { presence, hours })
            .await;
    }
    let bytes = ctx
        .present(rpc, |presence| pb::BeginCliSessionRequest { presence, hours })
        .await?;
    let mut session: pb::CliSession = decode(rpc, &bytes)?;
    let token = std::mem::take(&mut session.token);
    if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CliError::NoLoginToken);
    }
    login::save(&path, &token).map_err(|source| CliError::SaveLogin {
        path: path.clone(),
        source,
    })?;
    if let Principal::LoggedIn { .. } = ctx.principal {
        // The login this run was sent with is the one just replaced: an
        // empty id names it. Ending it is tidying up, so a failure is
        // left to its expiry.
        let _ = ctx
            .raw(
                rpc::cli_auth::END_CLI_SESSION,
                &pb::EndCliSessionRequest { id: String::new() },
            )
            .await;
    }
    Ok(Done::reply(as_json::encoded(
        rpc.response,
        &session.encode_to_vec(),
    )))
}

/// Ends this terminal's login in the daemon and forgets its token. With no
/// login saved here there's nothing to do; a login the daemon already
/// dropped (expired, or humatad restarted) is just forgotten.
pub(super) async fn logout<D: Daemon, C: Console>(ctx: &Ctx<'_, D, C>) -> Result<Done, CliError> {
    let Some(path) = login::path(ctx.environment).filter(|_| ctx.environment.login.is_some())
    else {
        ctx.note("This terminal isn't logged in.");
        return Ok(Done::reply(json!({})));
    };
    if ctx.flags.dry_run && matches!(ctx.principal, Principal::LoggedIn { .. }) {
        let request = pb::EndCliSessionRequest { id: String::new() };
        return ctx.change(rpc::cli_auth::END_CLI_SESSION, &request).await;
    }
    let ended = match ctx.principal {
        Principal::LoggedIn { .. } => {
            let request = pb::EndCliSessionRequest { id: String::new() };
            match ctx.json(rpc::cli_auth::END_CLI_SESSION, &request).await {
                Err(CliError::Daemon(
                    DaemonError::Refused { code, .. } | DaemonError::Status { code, .. },
                )) if code == NOT_FOUND || code == UNAUTHENTICATED => Ok(json!({})),
                other => other,
            }
        }
        // A session or script token wins over the login, so the daemon
        // wouldn't know which login to end: the token is still forgotten.
        Principal::Agent { .. } | Principal::Script { .. } | Principal::User => Ok(json!({})),
    };
    login::remove(&path).map_err(|source| CliError::RemoveLogin {
        path: path.clone(),
        source,
    })?;
    ended.map(Done::reply)
}
