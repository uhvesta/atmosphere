//! PlanForgetMachine and ForgetMachine (LD-11, LD-12, flow 26.23): for a
//! machine that is gone for good. The host never dials it: the plan lists
//! what is lost from the host's last pointers, and forgetting archives the
//! machine's sessions from the host's records, takes the machine out of every
//! channel and lets go of its link and reports. The sessions' part is the
//! SessionService's (`session_service/forget.rs`); this is the machine's.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use humata2_access::MachineActionKind;
use humata2_grpc::humata::v1 as pb;
use humata2_machines::ManageError;
use tonic::{Code, Request, Response, Status};

use super::{MachineSvc, Work, lock, machine_id, machine_of, status_of};
use crate::identity;
use crate::status;

/// The file beside `hosts.json` that keeps what each ForgetMachine key
/// answered, so a retry after the host restarts answers the same.
const ANSWERS_FILE: &str = "forget-answers.json";

/// How many answers the file keeps; the oldest go first.
const ANSWERS_KEPT: usize = 256;

/// How long a forget plan stays good.
const PLAN_LIFETIME: Duration = Duration::from_secs(10 * 60);

/// What forgetting a machine asks of its sessions. The SessionService does
/// it: the records are its.
#[tonic::async_trait]
pub(crate) trait Forgetting: Send + Sync + 'static {
    /// The work lost with each session recorded on `machine` and not yet
    /// archived, from the host's last pointers. Reads only.
    async fn lost_on(&self, machine: &str) -> Result<Vec<pb::LostWork>, Status>;

    /// Archives those sessions as lost with their machine, takes `machine`
    /// out of every channel and lets go of what the host kept of its
    /// reports, withdrawing each session's pending requests for `reason`.
    /// The sessions archived.
    async fn archive_lost(&self, machine: &str, reason: pb::WithdrawReason) -> Result<Vec<pb::SessionId>, Status>;

    /// Takes `machine` out of every channel that has it, and nothing else:
    /// what Remove machine does to the channels (D760).
    async fn take_out_of_channels(&self, machine: &str) -> Result<(), Status>;

    /// Ends each session recorded on `machine` and not yet archived through
    /// the machine's own session end, while its link is still up: what
    /// Remove machine does to them (D784). One the machine doesn't end is
    /// logged and left; nothing here fails the removal. Returns one line
    /// for each, naming it and how to end it.
    async fn end_sessions_on(&self, machine: &str) -> Vec<String>;

    /// What `machine` should let go of (D784): the sessions recorded on it
    /// whose archive says their machine was removed, and their channels
    /// that no longer list it. Reads only.
    async fn let_go_on(&self, machine: &str) -> Result<pb::LetGoOfHostRequest, Status>;
}

/// The forget plans handed out, the answers already given and the sessions'
/// side, set by `serve` once the SessionService exists.
#[derive(Default)]
pub(crate) struct Forgetful {
    hook: OnceLock<Arc<dyn Forgetting>>,
    plans: Mutex<HashMap<String, (String, Instant)>>,
    /// Held while the answers file is read or written.
    answers: Mutex<()>,
    /// Held while a forget's steps run, so a retry and the restart's own
    /// finish of the same forget take them one after the other.
    finishing: tokio::sync::Mutex<()>,
}

/// What one ForgetMachine key did, as the answers file keeps it. A started
/// entry (`done` false) is written before the first durable effect and holds
/// the sessions the plan listed, so a retry after a crash finishes the forget
/// without the plan token, which died with the process. A done entry is the
/// answer.
struct Answer {
    key: String,
    machine: String,
    archived: Vec<String>,
    done: bool,
}

fn answers_path(hosts_file: &Path) -> PathBuf {
    hosts_file.with_file_name(ANSWERS_FILE)
}

/// The answers kept, oldest first. A missing or unreadable file keeps none:
/// a retry then needs a new plan, which is safe as forgetting twice is.
fn read_answers(path: &Path) -> Vec<Answer> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Ok(serde_json::Value::Array(entries)) = serde_json::from_slice(&bytes) else {
        crate::log::say!("humatad2: {} is not a list of forget answers; retried forgets need a new plan", path.display());
        return Vec::new();
    };
    let text = |entry: &serde_json::Value, field: &str| entry.get(field).and_then(|value| value.as_str()).map(str::to_owned);
    entries
        .iter()
        .filter_map(|entry| {
            Some(Answer {
                key: text(entry, "key")?,
                machine: text(entry, "machine")?,
                archived: entry
                    .get("archived")?
                    .as_array()?
                    .iter()
                    .filter_map(|id| id.as_str().map(str::to_owned))
                    .collect(),
                // Entries from before started records are all answers.
                done: entry.get("done").and_then(|value| value.as_bool()).unwrap_or(true),
            })
        })
        .collect()
}

fn write_answers(path: &Path, answers: &[Answer]) -> Result<(), humata2_store::StoreError> {
    let entries: Vec<serde_json::Value> = answers
        .iter()
        .map(|answer| serde_json::json!({"key": answer.key, "machine": answer.machine, "archived": answer.archived, "done": answer.done}))
        .collect();
    let bytes = serde_json::to_vec_pretty(&entries).unwrap_or_default();
    humata2_store::write_atomic(path, &bytes)
}

impl Forgetful {
    pub(crate) fn set(&self, hook: Arc<dyn Forgetting>) {
        let _ = self.hook.set(hook);
    }

    fn hook(&self) -> Result<&Arc<dyn Forgetting>, Status> {
        self.hook.get().ok_or_else(|| {
            status::internal(
                "forgetting a machine is not wired up in this daemon",
                "",
            )
        })
    }

    fn make(&self, machine: &str) -> String {
        let token = format!("pf_{}", uuid::Uuid::new_v4().simple());
        let mut plans = lock(&self.plans);
        plans.retain(|_, (_, made)| made.elapsed() < PLAN_LIFETIME);
        plans.insert(token.clone(), (machine.to_owned(), Instant::now()));
        token
    }

    /// The sessions an earlier ForgetMachine of `machine` with `key` archived.
    fn answered(&self, hosts_file: &Path, key: &str, machine: &str) -> Option<Vec<pb::SessionId>> {
        let _held = lock(&self.answers);
        read_answers(&answers_path(hosts_file))
            .into_iter()
            .find(|answer| answer.key == key && answer.machine == machine && answer.done)
            .map(|answer| answer.archived.into_iter().map(|value| pb::SessionId { value }).collect())
    }

    /// The sessions the plan listed for a ForgetMachine of `machine` with
    /// `key` that started but never answered.
    fn started(&self, hosts_file: &Path, key: &str, machine: &str) -> Option<Vec<String>> {
        let _held = lock(&self.answers);
        read_answers(&answers_path(hosts_file))
            .into_iter()
            .find(|answer| answer.key == key && answer.machine == machine && !answer.done)
            .map(|answer| answer.archived)
    }

    /// Keeps what `key` answered. The forget is done either way, so a write
    /// that fails is only logged: a retry with the key then needs a new plan.
    fn remember(&self, hosts_file: &Path, key: String, machine: &str, archived: &[pb::SessionId]) {
        self.keep(hosts_file, key, machine, archived.iter().map(|id| id.value.clone()).collect(), true);
    }

    /// Keeps that `key` began forgetting `machine` with `planned` sessions
    /// to archive, before anything is archived. A write that fails is only
    /// logged: a crash after it then needs a new plan.
    fn begin(&self, hosts_file: &Path, key: String, machine: &str, planned: &[pb::SessionId]) {
        self.keep(hosts_file, key, machine, planned.iter().map(|id| id.value.clone()).collect(), false);
    }

    fn keep(&self, hosts_file: &Path, key: String, machine: &str, archived: Vec<String>, done: bool) {
        let _held = lock(&self.answers);
        let path = answers_path(hosts_file);
        let mut answers = read_answers(&path);
        answers.retain(|answer| answer.key != key);
        answers.push(Answer {
            key,
            machine: machine.to_owned(),
            archived,
            done,
        });
        let extra = answers.len().saturating_sub(ANSWERS_KEPT);
        answers.drain(..extra);
        if let Err(error) = write_answers(&path, &answers) {
            crate::log::say!("humatad2: keeping the forget answer in {}: {error}; a retry needs a new plan", path.display());
        }
    }

    /// The forgets that began and never answered, oldest first.
    fn unanswered(&self, hosts_file: &Path) -> Vec<Answer> {
        let _held = lock(&self.answers);
        read_answers(&answers_path(hosts_file)).into_iter().filter(|answer| !answer.done).collect()
    }

    /// Spends the plan `token` made for `machine`, once.
    fn spend(&self, token: &str, machine: &str) -> Result<(), Status> {
        let planned = lock(&self.plans)
            .remove(token)
            .filter(|(planned, made)| planned == machine && made.elapsed() < PLAN_LIFETIME);
        if planned.is_some() {
            return Ok(());
        }
        Err(status::failure(
            Code::FailedPrecondition,
            status::error(
                format!("the forget plan for {machine} is unknown, used or older than ten minutes"),
                "ask for a new plan (PlanForgetMachine), read what is lost, then forget with its token",
                pb::error::Kind::PlanStale(pb::PlanStale { changes: Vec::new() }),
            ),
        ))
    }
}

impl MachineSvc {
    /// What forgetting the machine loses, and the token ForgetMachine takes.
    /// Never dials the machine.
    pub(super) async fn plan_forget_of(
        &self,
        request: Request<pb::PlanForgetMachineRequest>,
    ) -> Result<Response<pb::ForgetMachinePlan>, Status> {
        let caller = identity::caller(&request)?;
        let name = machine_of(request.into_inner().id)?;
        self.require(&caller, &name, MachineActionKind::Forget)?;
        self.work
            .machines
            .get(&name)
            .map_err(|error| super::status_of(&error))?;
        let forgetting = &self.work.forgetting;
        let lost = forgetting.hook()?.lost_on(&name).await?;
        Ok(Response::new(pb::ForgetMachinePlan {
            machine: Some(machine_id(&name)),
            lost,
            token: Some(pb::PlanToken {
                value: forgetting.make(&name),
            }),
        }))
    }

    /// Forgets the machine the plan was made for: the user only, present.
    pub(super) async fn forget_of(
        &self,
        request: Request<pb::ForgetMachineRequest>,
    ) -> Result<Response<pb::ForgetMachineResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let name = machine_of(request.id)?;
        // Only the user forgets a machine: an agent is told so, whatever its
        // rule says, before any rule is read.
        let principal = crate::guard::principal(&caller);
        if self
            .guard
            .with_policy(|_, context| principal.present_user(context))
            .is_none()
        {
            return Err(status::refused(&humata2_access::Refusal::UserOnly {
                capability: Some(MachineActionKind::Forget.capability()),
            }));
        }
        self.require(&caller, &name, MachineActionKind::Forget)?;
        let forgetting = &self.work.forgetting;
        let key = request
            .idempotency_key
            .map(|key| key.value)
            .filter(|key| !key.is_empty());
        let hosts_file = self.work.machines.hosts_file();
        if let Some(key) = &key
            && let Some(archived) = forgetting.answered(hosts_file, key, &name)
        {
            return Ok(Response::new(pb::ForgetMachineResponse { archived }));
        }
        // A retry of a forget that began and was cut short finishes it: the
        // plan's token died with the process, but what it listed was kept.
        let begun = key.as_ref().and_then(|key| forgetting.started(hosts_file, key, &name));
        let mut planned = begun.clone().unwrap_or_default();
        // Every forget is kept before it archives anything, with the key
        // the caller gave or one of its own, so a restart can finish it
        // when no one asks again.
        let kept_as = key.clone().unwrap_or_else(|| format!("forget-{}", uuid::Uuid::new_v4().simple()));
        if begun.is_none() {
            let token = request
                .plan
                .map(|plan| plan.value)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| status::missing("plan"))?;
            let listed = forgetting.hook()?.lost_on(&name).await?;
            forgetting.spend(&token, &name)?;
            let ids: Vec<pb::SessionId> = listed.iter().filter_map(|lost| lost.session.clone()).collect();
            forgetting.begin(hosts_file, kept_as.clone(), &name, &ids);
            planned = ids.into_iter().map(|id| id.value).collect();
            // A kill here leaves the forget begun and nothing archived: the
            // restart finishes it.
            crate::fault::point("forget.begun");
        }
        let archived = self.work.finish_forget(hosts_file, kept_as, &name, planned, || {}).await?;
        Ok(Response::new(pb::ForgetMachineResponse { archived }))
    }
}

impl Work {
    /// `machine` is being removed: no channel lists it any more (D760).
    pub(super) async fn take_out_of_channels(&self, machine: &str) -> Result<(), Status> {
        self.forgetting.hook()?.take_out_of_channels(machine).await
    }

    /// `machine` is being removed and still answers: its sessions end there
    /// (D784). Remove machine only; Forget machine ends nothing, as that
    /// machine doesn't answer and its sessions keep running there.
    pub(super) async fn end_removed_sessions(&self, machine: &str) -> Vec<String> {
        match self.forgetting.hook() {
            Ok(hook) => hook.end_sessions_on(machine).await,
            Err(error) => {
                let left = format!(
                    "{machine}'s sessions couldn't be ended there: {}. They may still run on {machine}: add {machine} again, then end each holder from `humata session running --machine {machine}`",
                    error.message()
                );
                crate::log::say!("humatad2: {machine} is being removed, but {left}");
                vec![left]
            }
        }
    }

    /// `machine`'s removal is done: its sessions are archived as lost with
    /// it, and every request it raised is withdrawn as its machine removed,
    /// so a machine added later under the name shows none of them (D783).
    /// The host's id for a machine is still its name (D61, D776), so what is
    /// left keyed by it would attach to the next machine of that name.
    pub(super) async fn archive_removed(&self, machine: &str) -> Result<(), Status> {
        let archived = self
            .forgetting
            .hook()?
            .archive_lost(machine, pb::WithdrawReason::MachineRemoved)
            .await;
        archived.map(drop).and(self.withdraw_removed(machine).await)
    }

    /// What `machine` raised beyond its sessions' requests is withdrawn as
    /// its machine removed: those of sessions the host never recorded, any
    /// its journal brought for no session, and its Migrate items (D783).
    /// Removing and forgetting a machine both do it.
    async fn withdraw_removed(&self, machine: &str) -> Result<(), Status> {
        if let Some(upgrade) = self.upgrade.get() {
            upgrade.removed(machine).await;
        }
        match self.inbox.get() {
            Some(inbox) => inbox.withdraw_pending_from(machine, pb::WithdrawReason::MachineRemoved).await,
            None => Ok(()),
        }
    }

    /// The steps of a forget once its plan is spent and it is kept: archive
    /// the machine's sessions, take it out of the host's list and let go of
    /// its link, then keep the answer. Every step can run again, so a
    /// retry with the key and a restart both finish a forget cut short.
    async fn finish_forget(
        &self,
        hosts_file: &Path,
        key: String,
        name: &str,
        planned: Vec<String>,
        settled: impl FnOnce(),
    ) -> Result<Vec<pb::SessionId>, Status> {
        let forgetting = &self.forgetting;
        let _one_at_a_time = forgetting.finishing.lock().await;
        let mut archived = forgetting
            .hook()?
            .archive_lost(name, pb::WithdrawReason::SessionArchived)
            .await?;
        self.withdraw_removed(name).await?;
        // D784: a machine that answers lets go of the host's sessions now,
        // its holders still running; one that doesn't is told at its next
        // link (`follow_let_go`).
        self.tell_to_let_go(name).await;
        // What an earlier, cut short run archived is no longer listed by the
        // sessions' side; the answer is the same as the first call's.
        for value in planned.into_iter().rev() {
            if !archived.iter().any(|id| id.value == value) {
                archived.insert(0, pb::SessionId { value });
            }
        }
        crate::fault::point("forget.archived");
        // LD-12: this host forgets it without reaching it, so it is never
        // dialed again and adding it later makes a new machine. A retry
        // after this step finds it already gone.
        match self.machines.forget(name) {
            Ok(()) | Err(ManageError::NotFound { .. }) => {}
            Err(error) => return Err(status_of(&error)),
        }
        // A kill here leaves the machine gone from the list with its link
        // and the answer not yet let go of and kept: the restart finishes it.
        crate::fault::point("forget.machine_removed");
        self.link_down(name);
        self.forget_peer(name);
        self.forget_journal(name).await;
        // Before the answer is kept: whoever sees the answer then finds this
        // noted already.
        settled();
        forgetting.remember(hosts_file, key, name, &archived);
        Ok(archived)
    }
}

/// How long a machine has to let go of the host's sessions.
const LET_GO_DEADLINE: Duration = Duration::from_secs(30);

impl Work {
    /// Tells `machine`, if its link is up, to let go of the host's
    /// sessions the host archived as their machine removed, and of their
    /// channels (D784, D783). Its holders keep running there, under Not in
    /// a channel. Nothing is queued: a machine that is down, or doesn't
    /// answer, is told again at its next link. A failure is logged.
    async fn tell_to_let_go(&self, machine: &str) {
        let Ok(hook) = self.forgetting.hook() else {
            return;
        };
        let request = match hook.let_go_on(machine).await {
            Ok(request) if request.sessions.is_empty() => return,
            Ok(request) => request,
            Err(error) => {
                crate::log::say!(
                    "humatad2: couldn't read which of this host's sessions {machine} should let go of: {}; it is told again when it next links",
                    error.message()
                );
                return;
            }
        };
        let Ok(channel) = self.relay_links.channel(machine) else {
            return;
        };
        let mut client = pb::machine_service_client::MachineServiceClient::new(channel);
        let failed = match tokio::time::timeout(LET_GO_DEADLINE, client.let_go_of_host(Request::new(request))).await {
            Ok(Ok(_)) => return,
            Ok(Err(refused)) => status::error_of(&refused)
                .map(|error| format!("{} ({})", error.message, error.fix))
                .unwrap_or_else(|| refused.message().to_owned()),
            Err(_) => format!("it didn't answer within {} seconds", LET_GO_DEADLINE.as_secs()),
        };
        crate::log::say!(
            "humatad2: {machine} didn't let go of this host's sessions it was forgotten or removed with: {failed}. Its holders keep running there and it is told again when it next links; until then `humata session running --machine {machine}` may still list them in their old sessions"
        );
    }
}

/// For the daemon's life: each machine already up, then each that links,
/// is told to let go of the host's sessions it was forgotten or removed
/// with (D784), so a machine forgotten while offline lets go once it is
/// back. Spawn it once the SessionService's hook is set.
pub(crate) async fn follow_let_go(work: Arc<Work>) {
    let mut links = work.relay_links.watch_all();
    let up: Vec<String> = work
        .relay_links
        .states()
        .into_iter()
        .filter(|(_, state)| matches!(state, crate::host_relay::LinkState::Up { .. }))
        .map(|(machine, _)| machine)
        .collect();
    for machine in up {
        let work = work.clone();
        tokio::spawn(async move { work.tell_to_let_go(&machine).await });
    }
    while let Some((machine, state)) = links.next().await {
        if matches!(state, crate::host_relay::LinkState::Up { .. }) {
            let work = work.clone();
            tokio::spawn(async move { work.tell_to_let_go(&machine).await });
        }
    }
}

/// At start: finishes each forget the last run began and didn't answer, as
/// a retry with its key would, without the plan token that died with it.
/// Safe to repeat. Needs the SessionService's hook, so it runs once that
/// is set.
/// Each forget it will finish is named pending in the restart report when
/// this is called, before the future it returns runs (D241); spawn the
/// future before anything is served.
pub(crate) fn finish_forgets(
    work: Arc<Work>,
    started: Arc<crate::restart::Started>,
) -> impl std::future::Future<Output = ()> + Send + 'static {
    let hosts_file = work.machines.hosts_file().to_owned();
    let begun = work.forgetting.unanswered(&hosts_file);
    for forget in &begun {
        started.operations().record(
            &forget_operation(&forget.machine),
            "forget",
            "archiving its sessions",
            crate::restart::OperationState::Pending { waits_for: "the sessions' side of the host".to_owned() },
        );
    }
    settle_forgets(work, started, hosts_file, begun)
}

fn forget_operation(machine: &str) -> String {
    format!("forget:{machine}")
}

async fn settle_forgets(
    work: Arc<Work>,
    started: Arc<crate::restart::Started>,
    hosts_file: std::path::PathBuf,
    begun: Vec<Answer>,
) {
    use crate::restart::OperationState as State;
    let operations = started.operations();
    let operation_of = |machine: &str| forget_operation(machine);
    for forget in begun {
        let operation = operation_of(&forget.machine);
        let settled = || operations.record(&operation, "forget", "archiving its sessions", State::Finished);
        let finished = work.finish_forget(&hosts_file, forget.key, &forget.machine, forget.archived, settled).await;
        if let Err(error) = finished {
            let reason = format!(
                "{}; forget the machine again to finish",
                status::chain(&error)
            );
            crate::log::say!("humatad2: couldn't finish forgetting {}: {reason}", forget.machine);
            operations.record(&operation, "forget", "archiving its sessions", State::Stuck { reason });
        }
    }
}
