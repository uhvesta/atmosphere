//! The Inbox and the access rules over the daemon's InboxService and
//! AccessService: what is waiting, how it is answered, and what each
//! channel's agents may do.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_clients::DaemonError;
use humata2_clients::model::{
    AccessRule, Answer, AskedBy, Capability, ChannelId, CopyCard, DiffLine, Grant, GrantId, HolderNote,
    HolderSession, LineKind, MigrateCard, MigrateFailure, MigrateStep, MigrateStepState,
    MigratePhase, MigrateThing, Preview, Release, Request, RequestId, RequestKind, RuleSetting,
};
use humata2_grpc::humata::v1 as pb;

use crate::connection::{Shared, fresh_key};
use crate::map;

const PAGE_SIZE: u32 = 200;

/// What the app last saw of each request, for answering it.
#[derive(Default)]
pub(crate) struct Seen {
    /// Each pending request's revision and offered "Always allow" target,
    /// by request id.
    requests: BTreeMap<String, Offer>,
    /// Each capability's title as the Agent access tab shows it, by the
    /// proto's value.
    titles: BTreeMap<i32, String>,
}

#[derive(Clone)]
struct Offer {
    revision: Option<pb::Revision>,
    always: Option<pb::GrantTarget>,
}

// ---------------------------------------------------------------------------
// Pure mapping.

/// The app's capability id for a proto capability: its snake_case name.
fn capability_id(value: i32) -> Option<Capability> {
    let capability = pb::Capability::try_from(value).ok()?;
    if capability == pb::Capability::Unspecified {
        return None;
    }
    let name = capability.as_str_name();
    Some(Capability::new(
        name.strip_prefix("CAPABILITY_").unwrap_or(name).to_ascii_lowercase(),
    ))
}

/// The proto capability an app capability id names.
fn capability_value(capability: &Capability) -> Option<pb::Capability> {
    pb::Capability::from_str_name(&format!(
        "CAPABILITY_{}",
        capability.as_str().to_ascii_uppercase()
    ))
    .filter(|capability| *capability != pb::Capability::Unspecified)
}

/// A capability's row label: the daemon's title, else its name in words.
fn label(value: i32, capability: &Capability, titles: &BTreeMap<i32, String>) -> String {
    match titles.get(&value) {
        Some(title) if !title.is_empty() => title.clone(),
        _ => {
            let mut words = capability.as_str().replace('_', " ");
            if let Some(first) = words.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            words
        }
    }
}

fn setting(decision: i32) -> Option<RuleSetting> {
    match pb::Decision::try_from(decision).ok()? {
        pb::Decision::Allow => Some(RuleSetting::Allow),
        pb::Decision::Ask => Some(RuleSetting::Ask),
        pb::Decision::Never => Some(RuleSetting::Never),
        pb::Decision::Unspecified => None,
    }
}

fn decision(setting: RuleSetting) -> pb::Decision {
    match setting {
        RuleSetting::Allow => pb::Decision::Allow,
        RuleSetting::Ask => pb::Decision::Ask,
        RuleSetting::Never => pb::Decision::Never,
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
}

/// How long before `now` something happened, as people read it: "now",
/// "9m", "3h", "2d".
fn ago(now: i64, then: i64) -> String {
    let seconds = now.saturating_sub(then).max(0);
    match seconds {
        0..=59 => "now".to_owned(),
        60..=3599 => format!("{}m", seconds / 60),
        3600..=86_399 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}

fn path_text(path: &pb::RepoPath) -> String {
    let repo = path.repo.as_ref().map(map::repo_text).unwrap_or_default();
    match (repo.is_empty(), path.path.is_empty()) {
        (_, true) => repo,
        (true, false) => path.path.clone(),
        (false, false) => format!("{repo}/{}", path.path),
    }
}

/// A grant's target in words: "only under ledger/docs/".
// Old grants may still carry the deprecated BoardItem and Machine targets.
#[allow(deprecated)]
fn scope_text(target: Option<&pb::GrantTarget>) -> String {
    use pb::grant_target::Target as T;
    match target.and_then(|target| target.target.as_ref()) {
        Some(T::Path(path)) => match path.prefix.as_ref().map(path_text) {
            Some(prefix) if !prefix.is_empty() => format!("only under {prefix}"),
            _ => "anywhere in the repository".to_owned(),
        },
        Some(T::Session(session)) => format!("only session {}", session.value),
        Some(T::Network(network)) => format!("only {}:{}", network.host, network.port),
        Some(T::Board(_) | T::BoardItem(_)) => "only on that board".to_owned(),
        Some(T::AgentOnMachine(each)) => format!(
            "only {} on {}",
            each.agent.as_ref().map_or("that agent", |agent| agent.value.as_str()),
            each.machine.as_ref().map_or("that machine", |machine| machine.value.as_str())
        ),
        Some(T::RepoBranch(each)) => format!(
            "only {} on {}",
            each.repo.as_ref().map(map::repo_text).unwrap_or_default(),
            each.branch
        ),
        Some(T::Machine(machine)) => format!("only on {}", machine.value),
        Some(T::MachineAction(_)) => "only that machine action".to_owned(),
        None => String::new(),
    }
}

fn grant(grant: &pb::Grant, now: i64) -> Grant {
    Grant {
        id: GrantId::new(grant.id.as_ref().map(|id| id.value.clone()).unwrap_or_default()),
        session: map::session_id(grant.requester.as_ref()),
        scope: scope_text(grant.target.as_ref()),
        granted: grant
            .granted_at
            .as_ref()
            .map(|at| ago(now, at.seconds))
            .unwrap_or_default(),
    }
}

/// A channel's rules as the Agent access tab lists them, in the daemon's
/// order. A row the daemon gives no decision has no setting.
fn rules(reply: &pb::ChannelRules, titles: &BTreeMap<i32, String>, now: i64) -> Vec<AccessRule> {
    reply
        .rules
        .iter()
        .filter_map(|rule| {
            let capability = capability_id(rule.capability)?;
            Some(AccessRule {
                label: label(rule.capability, &capability, titles),
                setting: setting(rule.decision),
                grants: rule.grants.iter().map(|each| grant(each, now)).collect(),
                capability,
            })
        })
        .collect()
}

fn kind(request: &pb::InboxRequest) -> RequestKind {
    use pb::request_preview::Preview as P;
    let preview = request.preview.as_ref().and_then(|preview| preview.preview.as_ref());
    let edits_files = request.kind == pb::RequestKind::Files as i32;
    match preview {
        // An agent's file edit comes as the edit's text: it edits files.
        Some(P::Command(_)) if !edits_files => return RequestKind::Command,
        Some(P::Network(_)) => return RequestKind::Network,
        Some(P::Question(_)) => return RequestKind::Question,
        Some(P::Push(_)) => return RequestKind::Merge,
        Some(P::BoardChange(change)) if change.deletes_item => return RequestKind::Destroy,
        Some(P::Migrate(_)) => return RequestKind::Migrate,
        _ => {}
    }
    match pb::RequestKind::try_from(request.kind).unwrap_or(pb::RequestKind::Unspecified) {
        pb::RequestKind::Question => RequestKind::Question,
        pb::RequestKind::Bridge => RequestKind::Network,
        pb::RequestKind::Machine => RequestKind::Destroy,
        pb::RequestKind::Migrate => RequestKind::Migrate,
        pb::RequestKind::Files | pb::RequestKind::Board => RequestKind::FileEdit,
        pb::RequestKind::Session | pb::RequestKind::Channel | pb::RequestKind::Unspecified => {
            RequestKind::Command
        }
    }
}

/// An agent's file edit, its path then `-`/`+` lines or a patch, as diff
/// lines: the path and the patch's own headers grey, removals red, additions
/// green, every line whole.
fn edit_lines(edit: &str) -> Vec<DiffLine> {
    edit.lines()
        .enumerate()
        .map(|(at, line)| {
            let kind = if at == 0
                || line.starts_with("@@")
                || line.starts_with("***")
                || line.starts_with("+++")
                || line.starts_with("---")
            {
                LineKind::Hunk
            } else if line.starts_with('+') {
                LineKind::Added
            } else if line.starts_with('-') {
                LineKind::Removed
            } else {
                LineKind::Context
            };
            DiffLine { kind, old: None, new: None, text: line.to_owned() }
        })
        .collect()
}

/// An agent's provider import as its card shows it (MP-23, D792): what it
/// adds and replaces, every command it runs, then the file whole.
fn provider_import_text(import: &pb::ProviderImportPreview) -> String {
    let id = |provider: Option<&pb::ProviderId>| provider.map_or("", |id| id.value.as_str()).to_owned();
    let replaced: Vec<String> = import.replaces.iter().map(|each| each.value.clone()).collect();
    let mut lines: Vec<String> = import
        .providers
        .iter()
        .map(|provider| {
            let shown = id(provider.id.as_ref());
            let verb = if replaced.contains(&shown) { "Replaces" } else { "Adds" };
            format!("{verb} {shown} ({})", provider.label)
        })
        .collect();
    lines.push("Commands it runs on the Mac, or on its run_on machine, when a machine is made with it:".to_owned());
    lines.extend(
        import
            .commands
            .iter()
            .map(|command| format!("{} {}: {}", id(command.provider.as_ref()), command.action, command.command)),
    );
    let name = if import.file_name.is_empty() { "The file" } else { import.file_name.as_str() };
    lines.push(format!("{name}:"));
    lines.push(import.text.clone());
    lines.join("\n")
}

/// A repo of a new session or fork, with what its copy takes on disk when
/// it copies anything (26.5).
fn new_session_repo(repo: &pb::new_session_preview::Repo) -> String {
    let name = repo.repo.as_ref().map(map::repo_text).unwrap_or_default();
    if repo.clone_bytes == 0 {
        name
    } else {
        format!("{name}: copies {}", size_text(repo.clone_bytes))
    }
}

/// Bytes as people read them: `1.5 GB`.
#[allow(clippy::cast_precision_loss)]
fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{value:.1} {}", UNITS[unit]) }
}

fn preview(preview: Option<&pb::RequestPreview>) -> Preview {
    use pb::request_preview::Preview as P;
    let Some(preview) = preview.and_then(|preview| preview.preview.as_ref()) else {
        return Preview::Text(String::new());
    };
    match preview {
        P::Command(command) => Preview::Command(command.command.clone()),
        P::Typing(typing) => Preview::Text(typing.text.clone()),
        P::Message(message) => Preview::Text(message.text.clone()),
        P::Question(question) => Preview::Text(question.question.clone()),
        P::BoardChange(change) => Preview::Text(
            std::iter::once(change.item_title.clone())
                .chain(change.changes.iter().map(|each| {
                    format!("{}: {} -> {}", each.field, each.before, each.after)
                }))
                .chain(change.deletes_item.then(|| "Deletes the item".to_owned()))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        P::NewBoardItem(item) => Preview::Text(
            [item.title.as_str(), item.description.as_str()]
                .iter()
                .filter(|part| !part.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        P::FileAccess(access) => {
            Preview::Text(access.paths.iter().map(path_text).collect::<Vec<_>>().join("\n"))
        }
        P::NewSession(session) => Preview::Text(
            session
                .repos
                .iter()
                .map(new_session_repo)
                .chain((!session.usage_note.is_empty()).then(|| session.usage_note.clone()))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        P::CrossMachineCopy(copy) => Preview::CrossMachineCopy(CopyCard {
            from: map::machine_id(copy.source_machine.as_ref()),
            to: map::machine_id(copy.destination_machine.as_ref()),
            source_path: copy.source_path.clone(),
            destination_path: copy.destination_path.clone(),
            place: copy.destination_place.clone(),
            bytes: copy.bytes,
            files: copy.files,
        }),
        P::Move(each) => Preview::Text(format!(
            "{} -> {}",
            each.from.as_ref().map_or("", |machine| machine.value.as_str()),
            each.to.as_ref().map_or("", |machine| machine.value.as_str())
        )),
        P::Network(network) => Preview::Command(format!("{}:{}", network.host, network.port)),
        P::Push(push) => Preview::Text(
            push.repos
                .iter()
                .map(|each| {
                    format!(
                        "{} {} ({} commits)",
                        each.repo.as_ref().map(map::repo_text).unwrap_or_default(),
                        each.branch,
                        each.commits
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        P::AccessAhead(ahead) => Preview::Text(ahead.target_label.clone()),
        P::Migrate(migrate) => Preview::Migrate(migrate_card(migrate)),
        P::ProviderImport(import) => Preview::Text(provider_import_text(import)),
    }
}

fn migrate_thing(value: i32) -> MigrateThing {
    match pb::MigrateThing::try_from(value).unwrap_or(pb::MigrateThing::Unspecified) {
        pb::MigrateThing::Cli => MigrateThing::Cli,
        pb::MigrateThing::Store => MigrateThing::Store,
        pb::MigrateThing::Holders => MigrateThing::Holders,
        pb::MigrateThing::HostTools => MigrateThing::HostTools,
        pb::MigrateThing::DeskFolders => MigrateThing::DeskFolders,
        pb::MigrateThing::Daemon | pb::MigrateThing::Unspecified => MigrateThing::Daemon,
    }
}

fn release(build: Option<&pb::BuildInfo>) -> Release {
    Release {
        version: build.map(|build| build.version.clone()).unwrap_or_default(),
        bundle: build.map(|build| build.bundle.clone()).unwrap_or_default(),
    }
}

fn migrate_phase(migrate: &pb::MigratePreview) -> MigratePhase {
    match pb::MigratePhase::try_from(migrate.phase).unwrap_or(pb::MigratePhase::Unspecified) {
        pb::MigratePhase::Migrating => MigratePhase::Migrating,
        pb::MigratePhase::Failed => {
            let error = migrate.last_error.clone().unwrap_or_default();
            MigratePhase::Failed(MigrateFailure { message: error.message, fix: error.fix })
        }
        pb::MigratePhase::Waiting | pb::MigratePhase::Unspecified => MigratePhase::Waiting,
    }
}

/// A Migrate item's card; `migrated` is filled in by the request's answer.
fn migrate_card(migrate: &pb::MigratePreview) -> MigrateCard {
    MigrateCard {
        thing: migrate_thing(migrate.thing),
        machine: map::machine_id(migrate.machine.as_ref()),
        from: release(migrate.from.as_ref()),
        to: release(migrate.to.as_ref()),
        changes: migrate.changes.clone(),
        keeps_running: migrate.keeps_running.clone(),
        phase: migrate_phase(migrate),
        waiting_sessions: migrate
            .waiting_sessions
            .iter()
            .map(|session| map::session_id(Some(session)))
            .collect(),
        migrated: None,
        steps: migrate
            .steps
            .iter()
            .map(|step| MigrateStep {
                label: step.label.clone(),
                state: step_state(step.state),
            })
            .collect(),
        holders: migrate.holders.iter().map(holder_session).collect(),
    }
}

/// A step's state; unset or unknown reads as a step still to come.
fn step_state(state: i32) -> MigrateStepState {
    match pb::MigrateStepState::try_from(state).unwrap_or(pb::MigrateStepState::Unspecified) {
        pb::MigrateStepState::Doing => MigrateStepState::Doing,
        pb::MigrateStepState::Done => MigrateStepState::Done,
        pb::MigrateStepState::Failed => MigrateStepState::Failed,
        pb::MigrateStepState::Pending | pb::MigrateStepState::Unspecified => {
            MigrateStepState::Pending
        }
    }
}

/// A holder's session and its note; a shell tab has no session. An unset or
/// unknown note reads as staying until closed, which promises nothing.
fn holder_session(holder: &pb::HolderSession) -> HolderSession {
    let session = holder
        .session
        .as_ref()
        .filter(|session| !session.value.is_empty())
        .map(|session| map::session_id(Some(session)));
    let note = match pb::HolderNote::try_from(holder.note).unwrap_or(pb::HolderNote::Unspecified) {
        pb::HolderNote::RestartsWhenIdle => HolderNote::RestartsWhenIdle,
        pb::HolderNote::Restarting => HolderNote::Restarting,
        pb::HolderNote::NoIdleMoment => HolderNote::NoIdleMoment,
        pb::HolderNote::StaysUntilClosed | pb::HolderNote::Unspecified => {
            HolderNote::StaysUntilClosed
        }
    };
    HolderSession { session, title: holder.title.clone(), note }
}

/// When an answered request was answered, as people read it: "just now",
/// "2m ago".
fn answered_ago(request: &pb::InboxRequest, now: i64) -> Option<String> {
    let at = request.answered.as_ref()?.at.as_ref()?;
    Some(match ago(now, at.seconds).as_str() {
        "now" => "just now".to_owned(),
        span => format!("{span} ago"),
    })
}

/// How an answered request was answered.
fn answer(request: &pb::InboxRequest) -> Option<Answer> {
    use pb::request_answer::Answer as A;
    match request.answer.as_ref().and_then(|answer| answer.answer.as_ref()) {
        Some(A::Approve(_)) => Some(Answer::Approved),
        Some(A::Deny(deny)) => Some(Answer::Denied {
            reply: (!deny.note.is_empty()).then(|| deny.note.clone()),
        }),
        Some(A::Always(_)) => Some(Answer::Always {
            rule: request
                .always
                .as_ref()
                .map(|always| always.label.clone())
                .unwrap_or_default(),
        }),
        Some(A::Reply(reply)) => Some(Answer::Replied {
            text: reply.text.clone(),
        }),
        None => None,
    }
}

fn why_you(request: &pb::InboxRequest) -> String {
    if request.kind == pb::RequestKind::Question as i32 {
        return "The agent asked you a question.".to_owned();
    }
    match pb::Decision::try_from(request.rule_decision).ok() {
        Some(pb::Decision::Ask) => {
            "This channel's rule for this kind of action is Ask me, so the agent can't do it without you."
                .to_owned()
        }
        Some(pb::Decision::Never) => "This channel's rule for this kind of action is Never.".to_owned(),
        _ => "The agent can't do this without you.".to_owned(),
    }
}

fn request(request: &pb::InboxRequest, titles: &BTreeMap<i32, String>, now: i64) -> Request {
    let steps = request.recent_steps.clone();
    let asked_by = capability_id(request.capability).and_then(|capability| {
        Some(AskedBy {
            label: label(request.capability, &capability, titles),
            setting: setting(request.rule_decision)?,
            capability,
        })
    });
    let undo_note = (request.undo == pb::Undo::CannotBeUndone as i32)
        .then(|| "This can't be undone.".to_owned());
    Request {
        id: RequestId::new(request.id.as_ref().map(|id| id.value.clone()).unwrap_or_default()),
        session: map::session_id(request.requester.as_ref().and_then(|actor| actor.session.as_ref())),
        channel: ChannelId::new(
            request
                .channel
                .as_ref()
                .map(|id| id.value.clone())
                .unwrap_or_default(),
        ),
        kind: kind(request),
        title: request.action_title.clone(),
        reason: request.reason.clone(),
        preview: {
            let mut shown = preview(request.preview.as_ref());
            if kind(request) == RequestKind::FileEdit
                && let Preview::Command(edit) = &shown
            {
                let lines = edit_lines(edit);
                shown = Preview::Diff(lines);
            }
            if let (Preview::Migrate(card), Some(Answer::Approved)) = (&mut shown, answer(request)) {
                card.migrated = answered_ago(request, now);
            }
            shown
        },
        doing: steps.last().cloned().unwrap_or_default(),
        rule: request
            .always
            .as_ref()
            .map(|always| always.label.clone())
            .unwrap_or_default(),
        undoable: request.undo == pb::Undo::CanBeUndone as i32,
        asked: request
            .created_at
            .as_ref()
            .map(|at| ago(now, at.seconds))
            .unwrap_or_default(),
        answer: answer(request),
        operation: request.operation.clone(),
        steps,
        why_you: why_you(request),
        asked_by,
        undo_note,
        choices: match request.preview.as_ref().and_then(|preview| preview.preview.as_ref()) {
            Some(pb::request_preview::Preview::Question(question)) => question.choices.clone(),
            _ => Vec::new(),
        },
        on_screen: request.on_screen.as_ref().map(|on_screen| on_screen.tab as usize),
    }
}

/// The daemon's answer for one of the app's.
fn request_answer(answer: Answer, always: Option<pb::GrantTarget>) -> pb::RequestAnswer {
    use pb::request_answer::Answer as A;
    pb::RequestAnswer {
        answer: Some(match answer {
            Answer::Approved => A::Approve(pb::Approve {}),
            Answer::Denied { reply } => A::Deny(pb::Deny {
                note: reply.unwrap_or_default(),
            }),
            Answer::Replied { text } => A::Reply(pb::Reply { text }),
            Answer::Always { .. } => A::Always(pb::AlwaysLet { target: always }),
        }),
    }
}

// ---------------------------------------------------------------------------
// Calls.

impl Shared {
    /// Every capability's title, kept for the rows' labels.
    async fn read_titles(&self) -> Result<(), DaemonError> {
        let reply = self
            .access()
            .list_capabilities(pb::ListCapabilitiesRequest {})
            .await
            .map_err(|status| self.error(&status, "The access rules"))?
            .into_inner();
        self.state().inbox.titles = reply
            .capabilities
            .iter()
            .map(|info| (info.capability, info.title.clone()))
            .collect();
        Ok(())
    }

    /// Remembers what answering `mapped` needs, and maps it.
    pub(crate) fn map_request(&self, wire: &pb::InboxRequest) -> Request {
        let mapped = {
            let mut state = self.state();
            let id = wire.id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
            if wire.state == pb::RequestState::Pending as i32 {
                state.inbox.requests.insert(
                    id,
                    Offer {
                        revision: wire.revision.clone(),
                        always: wire.always.as_ref().and_then(|always| always.target.clone()),
                    },
                );
            } else {
                state.inbox.requests.remove(&id);
            }
            request(wire, &state.inbox.titles, now_secs())
        };
        mapped
    }

    /// The requests waiting for the user.
    pub(crate) async fn load_requests(&self) -> Result<Vec<Request>, DaemonError> {
        self.read_titles().await?;
        let mut requests = Vec::new();
        let mut token = String::new();
        loop {
            let reply = self
                .inbox()
                .list_requests(pb::ListRequestsRequest {
                    scope: Some(pb::InboxScope {
                        scope: Some(pb::inbox_scope::Scope::Global(pb::GlobalInbox {})),
                    }),
                    include_answered: false,
                    page: Some(pb::PageRequest {
                        page_size: PAGE_SIZE,
                        page_token: token,
                    }),
                })
                .await
                .map_err(|status| self.error(&status, "The Inbox"))?
                .into_inner();
            requests.extend(reply.requests.iter().map(|each| self.map_request(each)));
            token = reply.page.map(|page| page.next_page_token).unwrap_or_default();
            if token.is_empty() {
                return Ok(requests);
            }
        }
    }

    /// One channel's rules and the grants under them.
    pub(crate) async fn read_rules(&self, channel: &ChannelId) -> Result<Vec<AccessRule>, DaemonError> {
        let reply = self
            .access()
            .get_channel_rules(pb::GetChannelRulesRequest {
                channel: Some(pb::ChannelId {
                    value: channel.as_str().to_owned(),
                }),
            })
            .await
            .map_err(|status| self.error(&status, "The channel's access rules"))?
            .into_inner();
        Ok(self.map_rules(&reply))
    }

    pub(crate) fn map_rules(&self, reply: &pb::ChannelRules) -> Vec<AccessRule> {
        rules(reply, &self.state().inbox.titles, now_secs())
    }

    /// The rules of every channel in `channels`.
    pub(crate) async fn load_rules(
        &self,
        channels: &[ChannelId],
    ) -> Result<BTreeMap<ChannelId, Vec<AccessRule>>, DaemonError> {
        let mut all = BTreeMap::new();
        for channel in channels {
            all.insert(channel.clone(), self.read_rules(channel).await?);
        }
        Ok(all)
    }

    pub(crate) async fn answer_request(
        &self,
        id: RequestId,
        answer: Answer,
    ) -> Result<(), DaemonError> {
        const WHAT: &str = "Answering the request";
        let wire_id = pb::RequestId {
            value: id.as_str().to_owned(),
        };
        let mut offer = self.state().inbox.requests.get(id.as_str()).cloned();
        let needs_offer = matches!(answer, Answer::Always { .. });
        if offer.is_none() || (needs_offer && offer.as_ref().is_some_and(|o| o.always.is_none())) {
            let wire = self
                .inbox()
                .get_request(pb::GetRequestRequest {
                    id: Some(wire_id.clone()),
                })
                .await
                .map_err(|status| self.error(&status, WHAT))?
                .into_inner();
            let _ = self.map_request(&wire);
            offer = self.state().inbox.requests.get(id.as_str()).cloned();
        }
        let (revision, always) = match offer {
            Some(offer) => (offer.revision, offer.always),
            None => (None, None),
        };
        if needs_offer && always.is_none() {
            return Err(DaemonError::Refused {
                what: WHAT.to_owned(),
                reason: "the daemon offers no standing permission for this request; approve it once instead"
                    .to_owned(),
            });
        }
        self.inbox()
            .answer_request(pb::AnswerRequestRequest {
                idempotency_key: fresh_key(),
                id: Some(wire_id),
                answer: Some(request_answer(answer, always)),
                expected_revision: revision,
                presence: None,
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?;
        self.state().inbox.requests.remove(id.as_str());
        Ok(())
    }

    pub(crate) async fn change_rule(
        &self,
        channel: ChannelId,
        capability: Capability,
        setting: RuleSetting,
    ) -> Result<(), DaemonError> {
        const WHAT: &str = "Changing the access rule";
        let value = capability_value(&capability).ok_or_else(|| DaemonError::Refused {
            what: WHAT.to_owned(),
            reason: format!(
                "Humata's daemon has no access kind called {capability}; update Humata and the daemon to the same version"
            ),
        })?;
        self.access()
            .set_channel_rule(pb::SetChannelRuleRequest {
                idempotency_key: fresh_key(),
                channel: Some(pb::ChannelId {
                    value: channel.as_str().to_owned(),
                }),
                capability: value as i32,
                decision: decision(setting) as i32,
                expected_revision: None,
                presence: None,
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?;
        Ok(())
    }

    pub(crate) async fn revoke(&self, grant: GrantId) -> Result<(), DaemonError> {
        self.access()
            .revoke_grant(pb::RevokeGrantRequest {
                idempotency_key: fresh_key(),
                grant: Some(pb::GrantId {
                    value: grant.as_str().to_owned(),
                }),
                presence: None,
            })
            .await
            .map_err(|status| self.error(&status, "Removing the grant"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use humata2_clients::model::SessionId;

    #[test]
    fn every_capability_round_trips_by_name() {
        for value in 1..=200 {
            let Ok(capability) = pb::Capability::try_from(value) else {
                continue;
            };
            let id = capability_id(value).expect("a named capability has an id");
            assert_eq!(capability_value(&id), Some(capability));
        }
        assert_eq!(
            capability_id(pb::Capability::GlobalRead as i32),
            Some(Capability::new("global_read"))
        );
        assert_eq!(capability_id(0), None);
        assert_eq!(capability_value(&Capability::new("no_such_thing")), None);
    }

    #[test]
    fn a_row_without_a_decision_has_no_setting() {
        let reply = pb::ChannelRules {
            copy: None,
            rules: vec![
                pb::Rule {
                    capability: pb::Capability::GlobalRead as i32,
                    decision: pb::Decision::Ask as i32,
                    grants: vec![pb::Grant {
                        id: Some(pb::GrantId { value: "7".into() }),
                        target: Some(pb::GrantTarget {
                            target: Some(pb::grant_target::Target::Path(pb::PathScope {
                                prefix: Some(pb::RepoPath {
                                    repo: None,
                                    path: "ledger/docs/".into(),
                                }),
                                ..Default::default()
                            })),
                        }),
                        ..Default::default()
                    }],
                },
                pb::Rule {
                    capability: pb::Capability::ChannelRead as i32,
                    decision: pb::Decision::Unspecified as i32,
                    grants: Vec::new(),
                },
            ],
            ..Default::default()
        };
        let mapped = rules(&reply, &BTreeMap::new(), 0);
        assert_eq!(mapped[0].setting, Some(RuleSetting::Ask));
        assert_eq!(mapped[0].grants[0].id.as_str(), "7");
        assert_eq!(mapped[0].grants[0].scope, "only under ledger/docs/");
        assert_eq!(mapped[1].setting, None);
    }

    fn pending() -> pb::InboxRequest {
        pb::InboxRequest {
            id: Some(pb::RequestId { value: "rq_1".into() }),
            channel: Some(pb::ChannelId { value: "ch_1".into() }),
            kind: pb::RequestKind::Files as i32,
            state: pb::RequestState::Pending as i32,
            requester: Some(pb::AgentActor {
                session: Some(pb::SessionId { value: "se_1".into() }),
                ..Default::default()
            }),
            action_title: "Read 3 files".into(),
            reason: "to check the schema".into(),
            recent_steps: vec!["ls".into(), "cat a".into()],
            capability: pb::Capability::FilesReadOtherSession as i32,
            rule_decision: pb::Decision::Ask as i32,
            undo: pb::Undo::CannotBeUndone as i32,
            always: Some(pb::AlwaysOption {
                label: "Always allow reading".into(),
                target: None,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn a_pending_request_maps_every_field_it_has() {
        let mapped = request(&pending(), &BTreeMap::new(), 0);
        assert_eq!(mapped.id.as_str(), "rq_1");
        assert_eq!(mapped.session, SessionId::new("se_1"));
        assert_eq!(mapped.channel.as_str(), "ch_1");
        assert_eq!(mapped.kind, RequestKind::FileEdit);
        assert_eq!(mapped.title, "Read 3 files");
        assert_eq!(mapped.steps, vec!["ls", "cat a"]);
        assert_eq!(mapped.rule, "Always allow reading");
        assert!(!mapped.undoable);
        assert!(mapped.undo_note.is_some());
        assert_eq!(mapped.answer, None);
        let asked_by = mapped.asked_by.expect("the rule that asked");
        assert_eq!(asked_by.capability.as_str(), "files_read_other_session");
        assert_eq!(asked_by.setting, RuleSetting::Ask);
    }

    #[test]
    fn answers_become_the_daemons_answers() {
        use pb::request_answer::Answer as A;
        let target = pb::GrantTarget::default();
        assert!(matches!(
            request_answer(Answer::Approved, None).answer,
            Some(A::Approve(_))
        ));
        let denied = request_answer(Answer::Denied { reply: Some("no".into()) }, None);
        assert!(matches!(denied.answer, Some(A::Deny(ref deny)) if deny.note == "no"));
        let always = request_answer(
            Answer::Always { rule: String::new() },
            Some(target.clone()),
        );
        assert!(matches!(always.answer, Some(A::Always(ref each)) if each.target == Some(target)));
        let reply = request_answer(Answer::Replied { text: "yes".into() }, None);
        assert!(matches!(reply.answer, Some(A::Reply(ref each)) if each.text == "yes"));
    }

    #[test]
    fn an_agents_file_edit_is_an_edit_card_drawn_as_diff_lines() {
        let command = |kind: pb::RequestKind| {
            let mut wire = pending();
            wire.kind = kind as i32;
            wire.preview = Some(pb::RequestPreview {
                preview: Some(pb::request_preview::Preview::Command(pb::CommandPreview {
                    command: "/w/a.rs\n-let a = 1;\n+let a = 2;".into(),
                    ..Default::default()
                })),
            });
            request(&wire, &BTreeMap::new(), 0)
        };
        let edit = command(pb::RequestKind::Files);
        assert_eq!(edit.kind, RequestKind::FileEdit);
        let Preview::Diff(lines) = edit.preview else {
            panic!("a diff, not {:?}", edit.preview);
        };
        let kinds: Vec<LineKind> = lines.iter().map(|line| line.kind).collect();
        assert_eq!(kinds, [LineKind::Hunk, LineKind::Removed, LineKind::Added]);
        assert!(lines[0].text.contains("/w/a.rs"));
        assert!(lines[1].text.contains("let a = 1;") && lines[2].text.contains("let a = 2;"));

        // Any other agent tool stays a command.
        let other = command(pb::RequestKind::Session);
        assert_eq!(other.kind, RequestKind::Command);
        assert!(matches!(other.preview, Preview::Command(_)));
    }

    #[test]
    fn an_answered_request_says_how() {
        let mut wire = pending();
        wire.answer = Some(pb::RequestAnswer {
            answer: Some(pb::request_answer::Answer::Deny(pb::Deny { note: String::new() })),
        });
        assert_eq!(
            request(&wire, &BTreeMap::new(), 0).answer,
            Some(Answer::Denied { reply: None })
        );
    }

    #[test]
    fn a_fork_card_shows_what_each_copied_repo_takes_on_disk() {
        let repo = |name: &str, clone_bytes| pb::new_session_preview::Repo {
            repo: Some(pb::RepoRef { owner: "acme".into(), name: name.into() }),
            clone_bytes,
            ..Default::default()
        };
        let wire = pb::RequestPreview {
            preview: Some(pb::request_preview::Preview::NewSession(pb::NewSessionPreview {
                repos: vec![repo("api", 1_500_000_000), repo("web", 0)],
                ..Default::default()
            })),
        };
        let Preview::Text(text) = preview(Some(&wire)) else {
            panic!("a new session's card is text");
        };
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert!(lines[0].contains("api") && lines[0].contains("1.5 GB"), "{text}");
        assert!(lines[1].contains("web") && !lines[1].contains(" B"), "a repo that copies nothing names no size: {text}");
    }

    #[test]
    fn an_agents_provider_import_card_shows_every_command_and_the_whole_file() {
        let id = |value: &str| Some(pb::ProviderId { value: value.into() });
        let file = "[providers.corp]\nlabel = \"Corp\"\ncreate = 'corp-make {name}'\n";
        let wire = pb::RequestPreview {
            preview: Some(pb::request_preview::Preview::ProviderImport(pb::ProviderImportPreview {
                text: file.into(),
                file_name: "corp.toml".into(),
                providers: vec![pb::provider_import_preview::Provider { id: id("corp"), label: "Corp".into() }],
                commands: vec![
                    pb::provider_import_preview::Command {
                        provider: id("corp"),
                        action: "create".into(),
                        command: "corp-make {name}".into(),
                    },
                    pb::provider_import_preview::Command {
                        provider: id("corp"),
                        action: "connect".into(),
                        command: "corp-reach {name} -- {cmd}".into(),
                    },
                ],
                replaces: vec![],
            })),
        };
        let Preview::Text(text) = preview(Some(&wire)) else {
            panic!("a provider import's card is text");
        };
        assert!(text.contains(file), "the whole file: {text}");
        assert!(text.contains("corp-make {name}") && text.contains("corp-reach {name} -- {cmd}"), "{text}");
    }

    #[test]
    fn a_cross_machine_copy_maps_to_its_card_with_machines_and_measured_size() {
        let wire = pb::RequestPreview {
            preview: Some(pb::request_preview::Preview::CrossMachineCopy(pb::CrossMachineCopyPreview {
                source_machine: Some(pb::MachineId { value: "mac".into() }),
                source_path: "datasets".into(),
                destination_machine: Some(pb::MachineId { value: "gpu-box".into() }),
                destination_path: "shared".into(),
                bytes: 2_100_000_000,
                files: 42,
                destination_place: "ml shared".into(),
            })),
        };
        let Preview::CrossMachineCopy(card) = preview(Some(&wire)) else {
            panic!("a cross-machine copy has its own card");
        };
        assert_eq!((card.from.to_string(), card.to.to_string()), ("mac".to_owned(), "gpu-box".to_owned()));
        assert_eq!((card.bytes, card.files), (2_100_000_000, 42));
        assert_eq!(card.what(), "datasets");
        assert_eq!(card.place, "ml shared");
    }

    #[test]
    fn ages_read_as_short_spans() {
        assert_eq!(ago(100, 100), "now");
        assert_eq!(ago(1_000, 460), "9m");
        assert_eq!(ago(20_000, 0), "5h");
        assert_eq!(ago(200_000, 0), "2d");
    }
    #[test]
    fn a_failed_migrate_item_maps_its_card_and_an_approved_one_when_it_ended() {
        let preview = pb::RequestPreview {
            preview: Some(pb::request_preview::Preview::Migrate(pb::MigratePreview {
                thing: pb::MigrateThing::Cli as i32,
                machine: Some(pb::MachineId { value: "ma_2".into() }),
                from: Some(pb::BuildInfo { version: "2.0.4".into(), bundle: "aa".into(), ..Default::default() }),
                to: Some(pb::BuildInfo { version: "2.0.5".into(), bundle: "bb".into(), ..Default::default() }),
                changes: vec!["humata".into()],
                phase: pb::MigratePhase::Failed as i32,
                last_error: Some(pb::Error { message: "broke".into(), fix: "again".into(), ..Default::default() }),
                ..Default::default()
            })),
        };
        let mut wire = pending();
        wire.kind = pb::RequestKind::Migrate as i32;
        wire.preview = Some(preview);
        let mapped = request(&wire, &BTreeMap::new(), 120);
        assert_eq!(mapped.kind, RequestKind::Migrate);
        let Some(card) = mapped.migrate() else { panic!("a Migrate card") };
        assert_eq!(card.thing, MigrateThing::Cli);
        assert_eq!(card.machine.as_str(), "ma_2");
        assert_eq!(card.to.version, "2.0.5");
        assert_eq!(
            card.phase,
            MigratePhase::Failed(MigrateFailure { message: "broke".into(), fix: "again".into() })
        );
        assert_eq!(card.migrated, None);

        wire.answer = Some(pb::RequestAnswer { answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})) });
        wire.answered = Some(pb::Timestamped { at: Some(Default::default()), ..Default::default() });
        let done = request(&wire, &BTreeMap::new(), 120);
        assert_eq!(done.migrate().and_then(|card| card.migrated.clone()), Some("2m ago".to_owned()));
    }

    // D379: a Migrate card's steps and the sessions that hold the old
    // release, each with its note; a shell tab has no session.
    #[test]
    fn a_migrate_card_maps_its_steps_and_holders_with_their_notes() {
        use humata2_clients::model::{HolderNote, HolderSession, MigrateStep, MigrateStepState};
        let steps = [
            ("Check files", pb::MigrateStepState::Done),
            ("Copy the store", pb::MigrateStepState::Doing),
            ("Swap files", pb::MigrateStepState::Pending),
            ("Restart", pb::MigrateStepState::Failed),
        ];
        let preview = pb::RequestPreview {
            preview: Some(pb::request_preview::Preview::Migrate(pb::MigratePreview {
                thing: pb::MigrateThing::Holders as i32,
                machine: Some(pb::MachineId { value: "ma_2".into() }),
                steps: steps
                    .iter()
                    .map(|(label, state)| pb::MigrateStep {
                        label: (*label).into(),
                        state: *state as i32,
                    })
                    .collect(),
                holders: vec![
                    pb::HolderSession {
                        session: Some(pb::SessionId { value: "se_1".into() }),
                        title: "ledger".into(),
                        note: pb::HolderNote::RestartsWhenIdle as i32,
                    },
                    pb::HolderSession {
                        session: Some(pb::SessionId { value: "se_2".into() }),
                        title: "brin".into(),
                        note: pb::HolderNote::Restarting as i32,
                    },
                    pb::HolderSession {
                        session: Some(pb::SessionId { value: "se_3".into() }),
                        title: "copilot".into(),
                        note: pb::HolderNote::NoIdleMoment as i32,
                    },
                    // A shell tab: no session, set or not.
                    pb::HolderSession {
                        session: None,
                        title: "zsh".into(),
                        note: pb::HolderNote::StaysUntilClosed as i32,
                    },
                    pb::HolderSession {
                        session: Some(pb::SessionId { value: String::new() }),
                        title: "bash".into(),
                        note: pb::HolderNote::StaysUntilClosed as i32,
                    },
                ],
                waiting_sessions: vec![
                    pb::SessionId { value: "se_1".into() },
                    pb::SessionId { value: "se_2".into() },
                    pb::SessionId { value: "se_3".into() },
                ],
                ..Default::default()
            })),
        };
        let mut wire = pending();
        wire.kind = pb::RequestKind::Migrate as i32;
        wire.preview = Some(preview);
        let mapped = request(&wire, &BTreeMap::new(), 120);
        let Some(card) = mapped.migrate() else { panic!("a Migrate card") };

        assert_eq!(
            card.steps,
            vec![
                MigrateStep { label: "Check files".into(), state: MigrateStepState::Done },
                MigrateStep { label: "Copy the store".into(), state: MigrateStepState::Doing },
                MigrateStep { label: "Swap files".into(), state: MigrateStepState::Pending },
                MigrateStep { label: "Restart".into(), state: MigrateStepState::Failed },
            ]
        );
        assert_eq!(
            card.holders,
            vec![
                HolderSession {
                    session: Some(SessionId::new("se_1")),
                    title: "ledger".into(),
                    note: HolderNote::RestartsWhenIdle,
                },
                HolderSession {
                    session: Some(SessionId::new("se_2")),
                    title: "brin".into(),
                    note: HolderNote::Restarting,
                },
                HolderSession {
                    session: Some(SessionId::new("se_3")),
                    title: "copilot".into(),
                    note: HolderNote::NoIdleMoment,
                },
                HolderSession { session: None, title: "zsh".into(), note: HolderNote::StaysUntilClosed },
                HolderSession { session: None, title: "bash".into(), note: HolderNote::StaysUntilClosed },
            ]
        );
        // waiting_sessions (field 9) still maps as before.
        assert_eq!(
            card.waiting_sessions,
            vec![SessionId::new("se_1"), SessionId::new("se_2"), SessionId::new("se_3")]
        );
    }

    #[test]
    fn an_unset_or_unknown_step_state_reads_pending_and_no_holders_map_empty() {
        let preview = pb::RequestPreview {
            preview: Some(pb::request_preview::Preview::Migrate(pb::MigratePreview {
                thing: pb::MigrateThing::Cli as i32,
                machine: Some(pb::MachineId { value: "ma_2".into() }),
                // An unset or unknown state is a step still to come.
                steps: vec![
                    pb::MigrateStep { label: "Swap files".into(), state: 99 },
                    pb::MigrateStep { label: "Restart".into(), state: 0 },
                ],
                ..Default::default()
            })),
        };
        let mut wire = pending();
        wire.kind = pb::RequestKind::Migrate as i32;
        wire.preview = Some(preview);
        let card = request(&wire, &BTreeMap::new(), 120).migrate().cloned();
        let Some(card) = card else { panic!("a Migrate card") };
        assert!(card.holders.is_empty());
        assert_eq!(
            card.steps.iter().map(|step| step.state).collect::<Vec<_>>(),
            vec![humata2_clients::model::MigrateStepState::Pending; 2]
        );
    }
}
