//! A small world for the tests: a Mac, one more machine, two channels and a
//! few sessions in them.
//!
//! Sessions in channel Payments: `a` and `b` on the Mac, `c` and `d` on
//! the box. Session `x` is in channel Search, on the Mac.

#![allow(dead_code)]

use humata2_access::{
    AccessRequest, Agent, AnswerAt, BoardAct, BoardRef, Call, Capability, ChannelId, Context,
    Effect, Entry, Grant, MachineId, Place, Policy, PresenceMethod, PresentUser, Principal,
    Refusal, RepoPath, RequestId, Root, SessionId, SessionKind, SessionRef, Source, Target,
    Verdict,
};

pub fn mac() -> MachineId {
    MachineId::new("mac")
}

pub fn the_box() -> MachineId {
    MachineId::new("box")
}

pub fn payments() -> ChannelId {
    ChannelId::new("payments")
}

pub fn search() -> ChannelId {
    ChannelId::new("search")
}

fn session(id: &str, channel: ChannelId, machine: MachineId) -> SessionRef {
    SessionRef {
        id: SessionId::new(id),
        channel,
        machine,
    }
}

pub fn a() -> SessionRef {
    session("a", payments(), mac())
}

pub fn b() -> SessionRef {
    session("b", payments(), mac())
}

pub fn c() -> SessionRef {
    session("c", payments(), the_box())
}

pub fn d() -> SessionRef {
    session("d", payments(), the_box())
}

pub fn x() -> SessionRef {
    session("x", search(), mac())
}

pub fn agent(session: SessionRef) -> Agent {
    Agent {
        session,
        kind: SessionKind::Ordinary,
    }
}

pub fn as_agent(session: SessionRef) -> Principal {
    Principal::Agent(agent(session))
}

/// A policy knowing both channels, every rule at its default.
pub fn policy() -> Policy {
    let mut policy = Policy::new();
    policy.add_channel(payments()).expect("a new channel");
    policy.add_channel(search()).expect("a new channel");
    policy
}

pub fn on_mac() -> Context {
    Context::on_host(mac())
}

pub fn on_box_reachable() -> Context {
    Context::machine(the_box(), true)
}

pub fn on_box_away() -> Context {
    Context::machine(the_box(), false)
}

/// The user in the app.
pub fn user() -> PresentUser {
    Principal::App
        .present_user(&on_mac())
        .expect("the app is the user")
}

/// The user on the box with the approval passphrase, the Mac away.
pub fn user_on_box() -> PresentUser {
    Principal::Cli(PresenceMethod::ApprovalPassphrase)
        .present_user(&on_box_away())
        .expect("a passphrase shows presence")
}

pub fn path(value: &str) -> RepoPath {
    RepoPath::new(value).expect("a clean path")
}

/// A file in a session's clone, on the session's machine.
pub fn file_of(session: &SessionRef, file: &str) -> Place {
    Place {
        machine: session.machine.clone(),
        root: Root::Clone(session.clone()),
        path: path(file),
        entry: Entry::File,
    }
}

/// A folder in a session's clone, on the session's machine.
pub fn folder_of(session: &SessionRef, folder: &str) -> Place {
    Place {
        entry: Entry::Folder,
        ..file_of(session, folder)
    }
}

pub fn shared_on(machine: MachineId, channel: ChannelId, file: &str) -> Place {
    Place {
        machine,
        root: Root::Shared(channel),
        path: path(file),
        entry: Entry::File,
    }
}

pub fn read(place: Place) -> Call {
    Call::ReadFiles(place)
}

pub fn write(place: Place) -> Call {
    Call::WriteFiles(place)
}

pub fn use_on(capability: Capability, target: Target) -> Call {
    Call::Use { capability, target }
}

pub fn note_to(session: SessionRef) -> Call {
    use_on(Capability::SessionNote, Target::Session(session))
}

pub fn add_to_payments_board() -> Call {
    Call::Board {
        board: BoardRef::Channel(payments()),
        act: BoardAct::Add,
    }
}

pub fn check(policy: &Policy, principal: &Principal, call: &Call, context: &Context) -> Verdict {
    policy
        .check(principal, call, context)
        .expect("the call can be checked")
}

/// The request a verdict asks for; fails the test for any other verdict.
pub fn request(verdict: Verdict) -> AccessRequest {
    match verdict {
        Verdict::Ask(request) => request,
        other => panic!("expected an Inbox request, got {other:?}"),
    }
}

/// Asks with `agent`'s call and approves it with "Always let …" for the
/// offered target; returns the grant.
pub fn grant_for(policy: &mut Policy, agent: &Principal, call: &Call) -> Grant {
    let asked = request(check(policy, agent, call, &on_mac()));
    let offered = asked
        .always_let
        .clone()
        .expect("the request offers Always let");
    policy
        .approve(
            &user(),
            &RequestId::new("r1"),
            &asked,
            &humata2_access::Answer::AlwaysLet(offered),
        )
        .expect("the user approves")
        .grant
        .expect("Always let makes a grant")
}

/// What a test expects a check to decide.
#[derive(Debug, Clone)]
pub enum Expect {
    Allowed(Source, Effect),
    Ask(Capability, AnswerAt),
    WaitForHost,
    Refused(fn(&Refusal) -> bool),
}

impl Expect {
    pub fn met_by(&self, verdict: &Verdict) -> bool {
        match (self, verdict) {
            (Expect::Allowed(source, effect), Verdict::Allowed { source: s, effect: e }) => {
                source == s && effect == e
            }
            (Expect::Ask(capability, answer_at), Verdict::Ask(request)) => {
                &request.capability == capability && &request.answer_at == answer_at
            }
            (Expect::WaitForHost, Verdict::WaitForHost) => true,
            (Expect::Refused(is), Verdict::Refused(refusal)) => is(refusal),
            _ => false,
        }
    }
}

pub fn allowed(source: Source) -> Expect {
    Expect::Allowed(source, Effect::Apply)
}

pub fn asks(capability: Capability) -> Expect {
    Expect::Ask(capability, AnswerAt::Host)
}
