//! `humata hook <agent> permission` (HOOKS, D191): an agent's blocking
//! permission hook. Passes the payload the agent wrote on stdin to the
//! daemon, which raises an Inbox card and holds until it's answered, then
//! prints what the daemon says the hook prints. Always exits 0, and prints
//! nothing when there's no answer: no daemon, a refusal, a dropped stream
//! or the cap below. Empty output is "no decision" to every agent, so its
//! own prompt asks. `humata hook <agent> question` is the same for the
//! agent's own ask-user form: the card is an Inbox question (D761).
//! `humata hook <agent> conversation` tells the daemon which conversation
//! the agent's CLI is in, as the payload names it; it holds nothing.

use std::time::Duration;

use humata2_proto::humata::v1 as pb;
use prost::Message;

use crate::args::HookEventArg;
use crate::call::{Ctx, Done};
use crate::console::Console;
use crate::daemon::{Daemon, rpc};
use crate::error::CliError;

/// The CLI's own cap on the hold, above the daemon's 3590 s and below the
/// 3600 s the agents are told (D192), so the daemon always ends it first.
const HOLD: Duration = Duration::from_secs(3595);

/// The largest payload sent; a bigger one gets no decision.
const PAYLOAD_MAX: usize = 1024 * 1024;

pub(super) async fn hook<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    agent: &str,
    event: HookEventArg,
) -> Result<Done, CliError> {
    // A permission hook raises an Inbox card and answers the agent live, so
    // `--dry-run` is refused as usage before anything is read or sent.
    if ctx.flags.dry_run {
        return Err(CliError::HookDryRun);
    }
    let event = match event {
        HookEventArg::Permission => "permission",
        HookEventArg::Question => "question",
        HookEventArg::Conversation => "conversation",
        HookEventArg::Notification => "notification",
    };
    let Ok(payload) = ctx.console.stdin() else {
        return Ok(Done::printed());
    };
    if payload.len() > PAYLOAD_MAX {
        return Ok(Done::printed());
    }
    let request = pb::HookRequest {
        agent: agent.to_owned(),
        event: event.to_owned(),
        payload,
    };
    let answer = tokio::time::timeout(HOLD, async {
        let mut replies = ctx
            .daemon
            .server_stream(rpc::session::HOOK, request.encode_to_vec())
            .await
            .ok()?;
        while let Some(reply) = replies.recv().await {
            let reply = pb::HookReply::decode(reply.ok()?.as_slice()).ok()?;
            if !reply.keepalive {
                return Some(reply.stdout);
            }
        }
        None
    })
    .await;
    if let Ok(Some(stdout)) = answer
        && !stdout.is_empty()
    {
        ctx.console.write(&stdout);
    }
    Ok(Done::printed())
}
