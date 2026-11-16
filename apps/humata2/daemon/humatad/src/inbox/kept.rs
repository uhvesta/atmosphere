//! What the store keeps of the Inbox (D96), and the Inbox and grants a
//! restart brings back from it.
//!
//! A grant's target isn't saved as the policy holds it: what the agent
//! asked for is, and a restart asks the policy again what that names, so
//! a grant comes back exactly as narrow as it was made. A request whose
//! session or target is gone by then is withdrawn, and its grant dropped.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_access::{
    AccessRequest, Agent, Grant, GrantId, MachineId, Policy, PresenceMethod, RequestId,
    SessionKind, Verdict,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{InboxRecord, Store};
use humata2_store_proto::humata::store::v1 as stored;
use prost::Message;

use super::{Entry, Held, State, withdraw};
use crate::grant_target;
use crate::guard::Guard;
use crate::identity::AgentCaller;

/// The store's record of `entry` standing as `held`.
pub(super) fn record(entry: &Entry, held: &Held) -> InboxRecord {
    let grant = held.granted.clone().map(|granted| stored::InboxGrant {
        grant: held
            .grant
            .as_ref()
            .map(Message::encode_to_vec)
            .unwrap_or_default(),
        ..granted
    });
    let body = stored::InboxEntry {
        request: held.request.encode_to_vec(),
        keys: entry.keys.clone(),
        asked: entry
            .asked
            .as_ref()
            .map(Message::encode_to_vec)
            .unwrap_or_default(),
        grant,
        deferred: held.deferred,
    };
    let created = held.request.created_at.as_ref().map_or(0, |at| {
        at.seconds.saturating_mul(1000) + i64::from(at.nanos / 1_000_000)
    });
    InboxRecord {
        id: entry.id.clone(),
        session: entry.session.clone(),
        entry: body.encode_to_vec(),
        created_unix_ms: created,
        updated_unix_ms: unix_ms(SystemTime::now()),
    }
}

/// What the policy keeps beside `grant`, as it's saved.
pub(super) fn granted(grant: &Grant) -> stored::InboxGrant {
    let granted_by = match grant.granted_by {
        None => stored::GrantedBy::Unspecified,
        Some(PresenceMethod::ConfirmInApp) => stored::GrantedBy::ConfirmInApp,
        Some(PresenceMethod::CliLogin) => stored::GrantedBy::CliLogin,
        Some(PresenceMethod::ApprovalPassphrase) => stored::GrantedBy::ApprovalPassphrase,
    };
    stored::InboxGrant {
        grant: Vec::new(),
        granted_by: granted_by as i32,
        granted_on: grant.granted_on.to_string(),
        on_host: grant.on_host,
        revoked_unix_ms: 0,
    }
}

pub(super) fn unix_ms(at: SystemTime) -> i64 {
    let since = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
}

/// The Inbox the store kept, with every grant still standing put back in
/// the guard's policy. A request the store can't read is left out and
/// said in the log; the rest still load.
pub(super) fn restore(guard: &Guard, store: &Store, machine: &str) -> State {
    let mut state = State::default();
    let records = match store.inbox_requests() {
        Ok(records) => records,
        Err(error) => {
            crate::log::say!(
                "humatad2: warning: couldn't read the Inbox from the store, so it starts empty and no grant is in force: {}. Agents ask again; check the state folder's disk",
                crate::status::chain(&error)
            );
            return state;
        }
    };
    let mut grants = Vec::new();
    let mut next_grant = 1;
    for record in records {
        let Some((entry, grant, changed)) = restored(guard, store, machine, &record) else {
            crate::log::say!(
                "humatad2: warning: Inbox request {} in the store is unreadable, so it's left out; its agent can ask again",
                record.id
            );
            continue;
        };
        let held = entry.request.borrow().clone();
        if let Some(number) = held.grant.as_ref().and_then(grant_target::grant_number) {
            next_grant = next_grant.max(number.saturating_add(1));
        }
        if let Some(grant) = grant {
            grants.push(grant);
        }
        if changed && let Err(error) = store.put_inbox_request(&self::record(&entry, &held)) {
            crate::log::say!(
                "humatad2: warning: couldn't save Inbox request {} as withdrawn: {}",
                entry.id,
                crate::status::chain(&error)
            );
        }
        state.add(entry);
    }
    let installed = guard.with_policy(|policy, _| {
        let mut kept = policy.to_state();
        kept.grants.extend(grants);
        kept.next_grant = kept.next_grant.max(next_grant);
        Policy::from_state(kept).map(|restored| *policy = restored)
    });
    if let Err(error) = installed {
        crate::log::say!(
            "humatad2: warning: the grants in the store don't fit the access rules, so none is in force: {}. Agents ask again",
            crate::status::chain(&error)
        );
    }
    state
}

/// The entry `record` keeps, the grant to put back in the policy if one
/// still stands, and whether the entry changed on the way (withdrawn, or
/// its grant dropped). `None` when it can't be read.
fn restored(
    guard: &Guard,
    store: &Store,
    machine: &str,
    record: &InboxRecord,
) -> Option<(Entry, Option<Grant>, bool)> {
    let body = stored::InboxEntry::decode(record.entry.as_slice()).ok()?;
    let request = pb::InboxRequest::decode(body.request.as_slice()).ok()?;
    let asked = if body.asked.is_empty() {
        None
    } else {
        Some(pb::GrantTarget::decode(body.asked.as_slice()).ok()?)
    };
    let grant = match &body.grant {
        Some(granted) if !granted.grant.is_empty() => {
            Some(pb::Grant::decode(granted.grant.as_slice()).ok()?)
        }
        _ => None,
    };
    let deferred = body.deferred;
    let mut held = Held {
        request,
        grant,
        granted: body.grant.map(|granted| stored::InboxGrant {
            grant: Vec::new(),
            ..granted
        }),
        // A call waiting to run is gone with the process that held it.
        deferred,
        running: false,
        outcome: None,
    };
    let access = asked.clone().and_then(|target| {
        asked_again(
            guard,
            store,
            machine,
            &record.session,
            held.request.capability,
            target,
        )
    });
    let mut changed = false;
    if deferred && held.request.state == pb::RequestState::Pending as i32 {
        // Nothing is left that could run it, so it can't be approved.
        withdraw(&mut held.request, pb::WithdrawReason::CallEnded);
        changed = true;
    }
    if asked.is_some() && access.is_none() {
        if held.request.state == pb::RequestState::Pending as i32 {
            withdraw(&mut held.request, pb::WithdrawReason::SessionDeleted);
            changed = true;
        }
        if let Some(granted) = held
            .granted
            .as_mut()
            .filter(|granted| granted.revoked_unix_ms == 0)
        {
            granted.revoked_unix_ms = unix_ms(SystemTime::now());
            changed = true;
        }
    }
    let standing = match (&access, &held.grant, &held.granted) {
        (Some(access), Some(grant), Some(granted)) if granted.revoked_unix_ms == 0 => {
            policy_grant(&record.id, access, grant, granted)
        }
        _ => None,
    };
    let entry = Entry {
        id: record.id.clone(),
        session: record.session.clone(),
        access,
        asked,
        keys: body.keys,
        request: tokio::sync::watch::channel(held).0,
        run: std::sync::Mutex::new(None),
    };
    Some((entry, standing, changed))
}

/// What the policy asks the user for `session`'s ask-ahead on `target`,
/// worked out again as when it was asked. `None` once the session, or
/// what the target names, is gone.
fn asked_again(
    guard: &Guard,
    store: &Store,
    machine: &str,
    session: &str,
    capability: i32,
    target: pb::GrantTarget,
) -> Option<AccessRequest> {
    let capability = grant_target::capability_of(capability)?;
    let record = store.session(session).ok()??;
    let agent = AgentCaller::of(&record, machine);
    let call = grant_target::ask_ahead_call(store, machine, &agent, capability, target).ok()?;
    let asker = Agent {
        session: agent.session.clone(),
        kind: SessionKind::Ordinary,
    };
    guard.with_policy(|policy, context| {
        if policy.channel_rules(&asker.session.channel).is_none() {
            policy.add_channel(asker.session.channel.clone()).ok()?;
        }
        // No grant is back in the policy yet, so none covers it.
        match policy.check_ask_ahead(&asker, &call, context) {
            Ok(Verdict::Ask(asked)) => Some(asked),
            _ => None,
        }
    })
}

/// The grant `request`'s answer made, as the policy holds it.
fn policy_grant(
    request: &str,
    access: &AccessRequest,
    grant: &pb::Grant,
    granted: &stored::InboxGrant,
) -> Option<Grant> {
    let granted_by = match stored::GrantedBy::try_from(granted.granted_by).ok()? {
        stored::GrantedBy::Unspecified => None,
        stored::GrantedBy::ConfirmInApp => Some(PresenceMethod::ConfirmInApp),
        stored::GrantedBy::CliLogin => Some(PresenceMethod::CliLogin),
        stored::GrantedBy::ApprovalPassphrase => Some(PresenceMethod::ApprovalPassphrase),
    };
    let granted_at = grant.granted_at.as_ref().map_or(UNIX_EPOCH, |at| {
        let seconds = u64::try_from(at.seconds).unwrap_or(0);
        let nanos = u32::try_from(at.nanos).unwrap_or(0);
        UNIX_EPOCH + Duration::new(seconds, nanos)
    });
    Some(Grant {
        id: GrantId::new(grant_target::grant_number(grant)?),
        channel: access.channel.clone(),
        capability: access.capability,
        requester: access.requester.clone(),
        target: access.always_let.clone()?,
        granted_from: RequestId::new(request),
        granted_by,
        granted_on: MachineId::new(&granted.granted_on),
        on_host: granted.on_host,
        granted_at,
    })
}
