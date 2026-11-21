//! BROWSER: approval cards for actions in the user's own browser (read a
//! tab, list tabs, highlight guide, click, type, navigate), for claude,
//! codex, copilot and agy only. Reads may be allowed for 30 minutes per
//! session (in memory only; lost on daemon restart); writes are asked every
//! time.

use std::collections::{HashMap, HashSet};
use std::sync::PoisonError;
use std::time::{Duration, SystemTime};

use humata2_access::Refusal;
use humata2_browser_copilot::{ActionClass, BrowserAction, READ_GRANT_TTL_MS, SupportedAgent};
use humata2_daemon_ports::{AgentCaller, Caller};
use humata2_grpc::humata::v1 as pb;
use humata2_rpc_kit::helpers as status;
use pb::invalid_field::Problem;
use tonic::{Code, Request, Response, Status};

use super::{
    Entry, Held, Inbox, MAX_EXPLANATION, hold_through_gap, id_of, inbox_ask, new_id, settled,
};
use crate::ports::{caller as request_caller, now_timestamp};

/// In-memory per-session 30-minute browser read grants and one-time consumed
/// approval ids. Lost on daemon restart by design.
#[derive(Default)]
pub(super) struct BrowserGrants {
    /// `session_id` -> expiry timestamp of its 30-minute read grant.
    read_grants: HashMap<String, SystemTime>,
    /// Inbox request ids already consumed by `ExecuteBrowserAction`.
    used_approvals: HashSet<String>,
}

impl BrowserGrants {
    pub(super) fn grant_read(&mut self, session: &str, at: SystemTime) {
        let expires = at + Duration::from_millis(READ_GRANT_TTL_MS);
        self.read_grants.insert(session.to_owned(), expires);
    }

    pub(super) fn has_active_read_grant(&mut self, session: &str, now: SystemTime) -> bool {
        match self.read_grants.get(session).copied() {
            Some(expires) if now < expires => true,
            Some(_) => {
                self.read_grants.remove(session);
                false
            }
            None => false,
        }
    }

    pub(super) fn consume_once(&mut self, approval_id: &str) -> bool {
        self.used_approvals.insert(approval_id.to_owned())
    }
}

impl Inbox {
    /// Raises a browser read or write card, or answers a read at once under
    /// an active 30-minute grant of the caller's session.
    pub(super) async fn browser_approval(
        &self,
        request: Request<pb::RequestBrowserApprovalRequest>,
    ) -> Result<Response<pb::WaitResponse>, Status> {
        let caller = request_caller(&request)?;
        if matches!(caller, Caller::Script(_)) {
            return Err(self.guard.refused_status(&Refusal::ScriptTokenHasNoSession));
        }
        let Caller::Agent(agent) = &caller else {
            return Err(status::invalid_field(
                "action_json",
                Problem::NotAllowedHere,
                "only a session's agent asks for browser access; this connection is not a session agent",
                "ask from a Humata session tab running claude, codex, copilot or agy",
            ));
        };
        if SupportedAgent::parse(&agent.agent).is_err() {
            return Err(unsupported_agent_status(&agent.agent));
        }
        self.guard.check_inbox_ask(&caller, &inbox_ask(agent))?;

        let req = request.into_inner();
        let action = BrowserAction::from_json(&req.action_json).map_err(|error| {
            status::invalid_field(
                "action_json",
                Problem::BadFormat,
                format!("browser action JSON isn't valid: {error}"),
                "send a supported browser action JSON object (list_tabs, read_tab, highlight_guide, click, type, navigate)",
            )
        })?;
        let reason = req.reason.trim().to_owned();
        if reason.chars().count() > MAX_EXPLANATION {
            return Err(status::invalid_field(
                "reason",
                Problem::TooLong,
                format!(
                    "the reason is {} characters; it must be at most {MAX_EXPLANATION}",
                    reason.chars().count()
                ),
                "send a shorter reason saying why the task needs this browser action",
            ));
        }

        let is_write = ActionClass::classify(&action) == ActionClass::Write;
        let session = agent.session.id.to_string();
        let now = SystemTime::now();

        // Active 30-minute read grant answers reads immediately without raising a card.
        if !is_write
            && self
                .browser
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .has_active_read_grant(&session, now)
        {
            return Ok(Response::new(pb::WaitResponse {
                state: pb::RequestState::Approved as i32,
                answer: Some(pb::RequestAnswer {
                    answer: Some(pb::request_answer::Answer::AllowFor30Min(
                        pb::AllowFor30Min {},
                    )),
                }),
                ..Default::default()
            }));
        }

        let key = id_of(req.idempotency_key.map(|k| k.value))
            .map(|k| format!("{session}/{k}"));
        let mut state = self.state.lock().await;
        if let Some(made) = state.made_with(key.as_deref()) {
            return Ok(Response::new(made));
        }

        let id = new_id();
        let mut card = browser_card(&id, agent, &action, is_write, reason);
        card.working_on = self.working_on(agent);
        let held = Held::new(card);
        let reply = settled(&held);
        let entry = Entry::new(id, session, None, None, key, held.clone());
        self.commit(&entry, held, None, "save the browser approval request")
            .await?;
        self.tell(
            pb::Change::Created,
            &entry.session,
            &entry.request.borrow().request,
        );
        hold_through_gap(&self.asking, &entry.session, entry.request.subscribe());
        state.add(entry);
        Ok(Response::new(reply))
    }

    /// Records a 30-minute in-memory read grant for `entry.session` when a
    /// browser read card is answered with `AllowFor30Min`.
    pub(super) fn record_browser_answer(&self, entry: &Entry, allow_30m: bool, at: SystemTime) {
        if !allow_30m {
            return;
        }
        if is_read(&entry.request.borrow().request) == Some(true) {
            self.browser
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .grant_read(&entry.session, at);
        }
    }

    /// Verifies and consumes authorization for `session` to run one browser
    /// action with `action_key` and `is_write`.
    ///
    /// - For a read (`!is_write`): succeeds if `session` holds an active
    ///   30-minute read grant, or if `approval_id` names an approved
    ///   `BrowserRead` request of `session` with matching `action_key`
    ///   (consumed once).
    /// - For a write (`is_write`): requires `approval_id` naming an approved
    ///   `BrowserWrite` request of `session` with matching `action_key`
    ///   (consumed once; read grants never cover writes).
    pub async fn consume_browser_approval(
        &self,
        session: &str,
        approval_id: Option<&str>,
        action_key: &str,
        is_write: bool,
    ) -> Result<(), Status> {
        let now = SystemTime::now();
        if !is_write
            && self
                .browser
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .has_active_read_grant(session, now)
        {
            return Ok(());
        }

        let Some(id) = approval_id.filter(|id| !id.is_empty()) else {
            return Err(browser_denied(
                if is_write {
                    "this browser write action needs per-action approval in the Inbox"
                } else {
                    "this browser read action needs approval in the Inbox (no active 30-minute read grant)"
                },
                "ask the user first with RequestBrowserApproval and pass the approved request id",
            ));
        };

        let (mut current, pending_mirror) = {
            let state = self.state.lock().await;
            let Some(entry) = state
                .requests
                .iter()
                .find(|entry| entry.id == id && entry.session == session)
            else {
                return Err(browser_denied(
                    format!("browser approval {id} doesn't belong to this session"),
                    "ask the user first with RequestBrowserApproval from this session",
                ));
            };
            let current = entry.request.borrow().request.clone();
            // D891: when a remote machine's agent wakes from WaitForAnswer and
            // forwards ExecuteBrowserAction immediately, the machine's
            // JOURNAL_ANSWER entry may still be a few milliseconds behind on
            // the Uplink. Wait briefly on the mirrored entry's watch channel
            // without holding `self.state` so replay can settle it.
            let pending_mirror = (current.state == pb::RequestState::Pending as i32
                && current.journal.is_some())
            .then(|| entry.request.subscribe());
            (current, pending_mirror)
        };

        if let Some(mut watch) = pending_mirror {
            let _ = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let latest = watch.borrow_and_update().request.clone();
                    if latest.state != pb::RequestState::Pending as i32 {
                        current = latest;
                        break;
                    }
                    if watch.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await;
        }

        if current.state != pb::RequestState::Approved as i32 {
            return Err(browser_denied(
                format!("browser request {id} is not approved"),
                "wait until the user approves the request in the Inbox before running the action",
            ));
        }

        let preview = match current.preview.as_ref().and_then(|p| p.preview.as_ref()) {
            Some(pb::request_preview::Preview::BrowserRead(p)) if !is_write => p,
            Some(pb::request_preview::Preview::BrowserWrite(p)) if is_write => p,
            _ => {
                return Err(browser_denied(
                    format!("request {id} is not a matching browser approval for this action"),
                    "request approval for this exact action through RequestBrowserApproval",
                ));
            }
        };

        if preview.action_key != action_key {
            return Err(browser_denied(
                format!("request {id} was approved for a different browser action"),
                "run the exact action shown on the Inbox card, or request approval for the new action",
            ));
        }

        let mut grants = self
            .browser
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !grants.consume_once(id) {
            return Err(browser_denied(
                format!("browser approval {id} was already used"),
                "each browser approval is single-use; request a new approval in the Inbox",
            ));
        }

        Ok(())
    }
}

fn browser_card(
    id: &str,
    agent: &AgentCaller,
    action: &BrowserAction,
    is_write: bool,
    reason: String,
) -> pb::InboxRequest {
    let preview = pb::BrowserPreview {
        url: action.target_url().unwrap_or_default().to_owned(),
        action_summary: action.action_summary(),
        target_preview: action.target_preview().unwrap_or_default(),
        // Server-clamped: writes never support 30-minute grants.
        supports_30m_grant: !is_write,
        action: action.kind().to_owned(),
        action_key: action.action_key(),
        reason: reason.clone(),
    };
    let (kind, preview_variant) = if is_write {
        (
            pb::RequestKind::BrowserWrite,
            pb::request_preview::Preview::BrowserWrite(preview),
        )
    } else {
        (
            pb::RequestKind::BrowserRead,
            pb::request_preview::Preview::BrowserRead(preview),
        )
    };
    let operation = format!("browser.{}", action.kind());
    let who = if agent.title.is_empty() {
        agent.agent.as_str()
    } else {
        agent.title.as_str()
    };
    pb::InboxRequest {
        id: Some(pb::RequestId {
            value: id.to_owned(),
        }),
        channel: Some(pb::ChannelId {
            value: agent.session.channel.to_string(),
        }),
        kind: kind as i32,
        state: pb::RequestState::Pending as i32,
        requester: Some(agent.actor()),
        operation,
        action_title: format!("{who}: {}", action.action_summary()),
        reason,
        preview: Some(pb::RequestPreview {
            preview: Some(preview_variant),
        }),
        capability: pb::Capability::InboxAsk as i32,
        always: None,
        created_at: Some(now_timestamp()),
        revision: Some(pb::Revision { value: 1 }),
        ..Default::default()
    }
}

fn unsupported_agent_status(agent: &str) -> Status {
    browser_denied(
        format!(
            "agent `{agent}` isn't allowed to use the browser co-pilot (only claude, codex, copilot and agy are supported)"
        ),
        "use a claude, codex, copilot or agy session for browser actions",
    )
}

fn browser_denied(message: impl Into<String>, fix: impl Into<String>) -> Status {
    status::failure(
        Code::PermissionDenied,
        status::error(
            message.into(),
            fix.into(),
            pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::Unspecified as i32,
                capability: pb::Capability::InboxAsk as i32,
                request: None,
                user_note: String::new(),
            }),
        ),
    )
}

/// `Some(true)` for a browser read card, `Some(false)` for a browser write
/// card, `None` for any other request.
pub(super) fn is_read(request: &pb::InboxRequest) -> Option<bool> {
    use pb::request_preview::Preview as P;
    match request.preview.as_ref()?.preview.as_ref()? {
        P::BrowserRead(_) => Some(true),
        P::BrowserWrite(_) => Some(false),
        _ => None,
    }
}

/// Clamps `supports_30m_grant` on a browser preview (`true` for reads,
/// `false` for writes) and clears `always`.
pub(super) fn clamp_preview(request: &mut pb::InboxRequest) {
    use pb::request_preview::Preview as P;
    if let Some(preview) = request.preview.as_mut().and_then(|p| p.preview.as_mut()) {
        match preview {
            P::BrowserRead(p) => {
                p.supports_30m_grant = true;
                request.always = None;
            }
            P::BrowserWrite(p) => {
                p.supports_30m_grant = false;
                request.always = None;
            }
            _ => {}
        }
    }
}

/// Returns `false` if `request` is a browser card carrying a forbidden answer
/// (`AllowFor30Min` on a write, or `Always` on any browser card).
pub(super) fn valid_replayed_answer(request: &pb::InboxRequest) -> bool {
    let Some(read) = is_read(request) else {
        return true;
    };
    let Some(answer) = request.answer.as_ref().and_then(|a| a.answer.as_ref()) else {
        return true;
    };
    use pb::request_answer::Answer as A;
    match answer {
        A::Approve(_) | A::Deny(_) => true,
        A::AllowFor30Min(_) => read,
        A::Always(_) | A::Reply(_) | A::TodoAction(_) | A::TodoReview(_) => false,
    }
}

/// What answering browser card `id` with `answer` does: Allow once and Deny
/// on every browser card, Allow 30 min only on a read. Always allow, a
/// reply and to-do answers never fit.
pub(super) fn decide(
    id: &str,
    read: bool,
    answer: pb::request_answer::Answer,
) -> Result<(pb::RequestState, pb::request_answer::Answer), Status> {
    use pb::request_answer::Answer as A;
    match answer {
        answer @ A::Deny(_) => Ok((pb::RequestState::Denied, answer)),
        answer @ A::Approve(_) => Ok((pb::RequestState::Approved, answer)),
        answer @ A::AllowFor30Min(_) if read => Ok((pb::RequestState::Approved, answer)),
        A::AllowFor30Min(_) => Err(status::invalid_field(
            "answer",
            Problem::NotAllowedHere,
            format!(
                "request {id} asks to click, type or open a page in your browser: that is asked every time and can't be allowed for 30 minutes"
            ),
            "allow it once, or deny it",
        )),
        A::Always(_) => Err(status::invalid_field(
            "answer",
            Problem::NotAllowedHere,
            format!("request {id} is a browser request: browser access is never allowed always"),
            if read {
                "allow it once, allow it for 30 minutes, or deny it"
            } else {
                "allow it once, or deny it"
            },
        )),
        A::Reply(_) | A::TodoAction(_) | A::TodoReview(_) => Err(status::invalid_field(
            "answer",
            Problem::NotAllowedHere,
            format!("request {id} is a browser request: it takes allow or deny"),
            if read {
                "allow it once, allow it for 30 minutes, or deny it"
            } else {
                "allow it once, or deny it"
            },
        )),
    }
}

#[cfg(test)]
mod tests {
    use humata2_grpc::humata::v1::inbox_service_server::InboxService;
    use humata2_store::SessionRecord;

    use super::*;
    use crate::inbox::tests::{agent_req, id, test_inbox, user_req};

    fn agent_req_kind<T>(msg: T, session: &str, kind: &str) -> Request<T> {
        let mut req = agent_req(msg, session);
        if let Some(Caller::Agent(agent)) = req.extensions_mut().get_mut::<Caller>() {
            agent.agent = kind.to_owned();
        }
        req
    }

    fn seed_session(inbox: &Inbox, session: &str, agent: &str) {
        inbox
            .store
            .put_session(&SessionRecord {
                id: session.to_owned(),
                channel: "ch_1".to_owned(),
                machine: String::new(),
                title: "Session".to_owned(),
                agent: agent.to_owned(),
                folder: std::path::PathBuf::from("/tmp/x"),
                created_unix_ms: 0,
            })
            .unwrap();
    }

    fn list_tabs_json() -> String {
        r#"{"type":"list_tabs"}"#.to_owned()
    }

    fn click_json() -> String {
        r#"{"type":"click","tab_id":1,"url":"https://app.example.com/settings","selector":"button#save","element_preview":"Save"}"#.to_owned()
    }

    #[tokio::test]
    async fn unsupported_agents_are_refused_browser_approval() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = test_inbox(&dir);
        seed_session(&inbox, "se_opencode", "opencode");
        seed_session(&inbox, "se_zsh", "zsh");

        for (session, kind) in [("se_opencode", "opencode"), ("se_zsh", "zsh")] {
            let err = inbox
                .request_browser_approval(agent_req_kind(
                    pb::RequestBrowserApprovalRequest {
                        action_json: list_tabs_json(),
                        reason: "check tabs".to_owned(),
                        ..Default::default()
                    },
                    session,
                    kind,
                ))
                .await
                .unwrap_err();
            assert_eq!(err.code(), Code::PermissionDenied, "{kind}");
        }
    }

    #[tokio::test]
    async fn read_30m_grant_is_per_session_and_reused_for_later_reads_but_never_writes() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = test_inbox(&dir);
        seed_session(&inbox, "se_1", "claude");
        seed_session(&inbox, "se_2", "claude");

        let pending = inbox
            .request_browser_approval(agent_req(
                pb::RequestBrowserApprovalRequest {
                    action_json: list_tabs_json(),
                    reason: "inspect tabs".to_owned(),
                    ..Default::default()
                },
                "se_1",
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(pending.state, pb::RequestState::Pending as i32);
        let rq_id = pending.id.unwrap().value;

        let card = inbox
            .get_request(user_req(pb::GetRequestRequest { id: id(&rq_id) }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(card.kind, pb::RequestKind::BrowserRead as i32);
        let Some(pb::request_preview::Preview::BrowserRead(preview)) =
            card.preview.and_then(|p| p.preview)
        else {
            panic!("expected BrowserRead preview");
        };
        assert!(preview.supports_30m_grant);

        // User grants 30-min read access for se_1.
        inbox
            .answer_request(user_req(pb::AnswerRequestRequest {
                id: id(&rq_id),
                answer: Some(pb::RequestAnswer {
                    answer: Some(pb::request_answer::Answer::AllowFor30Min(
                        pb::AllowFor30Min {},
                    )),
                }),
                ..Default::default()
            }))
            .await
            .unwrap();

        // A second read in se_1 is approved immediately without raising a card.
        let read_tab = r#"{"type":"read_tab"}"#.to_owned();
        let fast = inbox
            .request_browser_approval(agent_req(
                pb::RequestBrowserApprovalRequest {
                    action_json: read_tab,
                    reason: "read active tab".to_owned(),
                    ..Default::default()
                },
                "se_1",
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(fast.state, pb::RequestState::Approved as i32);
        assert!(matches!(
            fast.answer.and_then(|a| a.answer),
            Some(pb::request_answer::Answer::AllowFor30Min(_))
        ));

        // se_2 does NOT inherit se_1's read grant.
        let other = inbox
            .request_browser_approval(agent_req(
                pb::RequestBrowserApprovalRequest {
                    action_json: list_tabs_json(),
                    reason: "inspect tabs".to_owned(),
                    ..Default::default()
                },
                "se_2",
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(other.state, pb::RequestState::Pending as i32);

        // A write in se_1 still requires per-action approval despite the active read grant.
        let write_wait = inbox
            .request_browser_approval(agent_req(
                pb::RequestBrowserApprovalRequest {
                    action_json: click_json(),
                    reason: "save settings".to_owned(),
                    ..Default::default()
                },
                "se_1",
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(write_wait.state, pb::RequestState::Pending as i32);
        let write_id = write_wait.id.unwrap().value;

        let write_card = inbox
            .get_request(user_req(pb::GetRequestRequest { id: id(&write_id) }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(write_card.kind, pb::RequestKind::BrowserWrite as i32);
        let Some(pb::request_preview::Preview::BrowserWrite(wp)) =
            write_card.preview.and_then(|p| p.preview)
        else {
            panic!("expected BrowserWrite preview");
        };
        assert!(!wp.supports_30m_grant);
        assert_eq!(wp.url, "https://app.example.com/settings");
    }

    #[tokio::test]
    async fn browser_write_rejects_allow_30m_and_always_and_is_skipped_by_approve_all() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = test_inbox(&dir);
        seed_session(&inbox, "se_1", "agy");

        let read_rq = inbox
            .request_browser_approval(agent_req_kind(
                pb::RequestBrowserApprovalRequest {
                    action_json: list_tabs_json(),
                    reason: "list".to_owned(),
                    ..Default::default()
                },
                "se_1",
                "agy",
            ))
            .await
            .unwrap()
            .into_inner()
            .id
            .unwrap()
            .value;

        let write_rq = inbox
            .request_browser_approval(agent_req_kind(
                pb::RequestBrowserApprovalRequest {
                    action_json: click_json(),
                    reason: "click".to_owned(),
                    ..Default::default()
                },
                "se_1",
                "agy",
            ))
            .await
            .unwrap()
            .into_inner()
            .id
            .unwrap()
            .value;

        // AllowFor30Min on BrowserWrite is rejected.
        let err = inbox
            .answer_request(user_req(pb::AnswerRequestRequest {
                id: id(&write_rq),
                answer: Some(pb::RequestAnswer {
                    answer: Some(pb::request_answer::Answer::AllowFor30Min(
                        pb::AllowFor30Min {},
                    )),
                }),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument);

        // AlwaysLet on both BrowserRead and BrowserWrite is rejected.
        for rq in [&read_rq, &write_rq] {
            let err = inbox
                .answer_request(user_req(pb::AnswerRequestRequest {
                    id: id(rq),
                    answer: Some(pb::RequestAnswer {
                        answer: Some(pb::request_answer::Answer::Always(pb::AlwaysLet::default())),
                    }),
                    ..Default::default()
                }))
                .await
                .unwrap_err();
            assert_eq!(err.code(), Code::InvalidArgument);
        }

        // Approve all approves BrowserRead once and skips BrowserWrite.
        let bulk = inbox
            .answer_all(user_req(pb::AnswerAllRequest {
                ids: vec![
                    pb::RequestId { value: read_rq.clone() },
                    pb::RequestId { value: write_rq.clone() },
                ],
                approve: true,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(bulk.answered.len(), 1);
        assert_eq!(bulk.answered[0].id.as_ref().map(|i| i.value.as_str()), Some(read_rq.as_str()));
        assert_eq!(bulk.skipped.len(), 1);
        assert_eq!(bulk.skipped[0].value, write_rq);

        // One-use approval verification: consuming write_rq before approval fails,
        // after approval succeeds once, and a second use fails.
        let click = BrowserAction::from_json(&click_json()).unwrap();
        let key = click.action_key();
        assert!(
            inbox
                .consume_browser_approval("se_1", Some(&write_rq), &key, true)
                .await
                .is_err()
        );

        inbox
            .answer_request(user_req(pb::AnswerRequestRequest {
                id: id(&write_rq),
                answer: Some(pb::RequestAnswer {
                    answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
                }),
                ..Default::default()
            }))
            .await
            .unwrap();

        // Wrong action key fails.
        assert!(
            inbox
                .consume_browser_approval("se_1", Some(&write_rq), "wrong-key", true)
                .await
                .is_err()
        );
        // Matching key succeeds once.
        inbox
            .consume_browser_approval("se_1", Some(&write_rq), &key, true)
            .await
            .unwrap();
        // Reusing the same approval fails.
        assert!(
            inbox
                .consume_browser_approval("se_1", Some(&write_rq), &key, true)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn replayed_allow_30m_grants_host_read_access_and_pending_mirror_waits_for_answer() {
        use prost::Message;

        let dir = tempfile::tempdir().unwrap();
        let inbox = std::sync::Arc::new(test_inbox(&dir));
        inbox
            .store
            .put_session(&SessionRecord {
                id: "se_rem".to_owned(),
                channel: "ch_1".to_owned(),
                machine: "ma_box".to_owned(),
                title: "Remote".to_owned(),
                agent: "claude".to_owned(),
                folder: std::path::PathBuf::from("/tmp/rem"),
                created_unix_ms: 0,
            })
            .unwrap();

        let action = BrowserAction::from_json(&list_tabs_json()).unwrap();
        let key = action.action_key();
        let pending_card = pb::InboxRequest {
            id: Some(pb::RequestId {
                value: "rq_rem_1".to_owned(),
            }),
            channel: Some(pb::ChannelId {
                value: "ch_1".to_owned(),
            }),
            kind: pb::RequestKind::BrowserRead as i32,
            state: pb::RequestState::Pending as i32,
            requester: Some(pb::AgentActor {
                session: Some(pb::SessionId {
                    value: "se_rem".to_owned(),
                }),
                agent: Some(pb::AgentId {
                    value: "claude".to_owned(),
                }),
                machine: None,
                session_title: "Remote".to_owned(),
            }),
            operation: "browser.list_tabs".to_owned(),
            preview: Some(pb::RequestPreview {
                preview: Some(pb::request_preview::Preview::BrowserRead(
                    pb::BrowserPreview {
                        url: String::new(),
                        action_summary: action.action_summary(),
                        target_preview: String::new(),
                        supports_30m_grant: true,
                        action: action.kind().to_owned(),
                        action_key: key.clone(),
                        reason: "inspect".to_owned(),
                    },
                )),
            }),
            ..Default::default()
        };

        let req_entry = pb::JournalEntry {
            seq: 1,
            operation: humata2_svc_away::JOURNAL_REQUEST.to_owned(),
            principal: Some(pb::AgentPrincipal {
                session: Some(pb::SessionId {
                    value: "se_rem".to_owned(),
                }),
                pid: 0,
            }),
            request: pending_card.encode_to_vec(),
            ..Default::default()
        };
        assert_eq!(
            inbox.replay("ma_box", &req_entry).await,
            crate::inbox::replay::Replayed::Applied
        );

        // Start consume_browser_approval while the mirrored card is still Pending
        // (simulating ExecuteBrowserAction racing ahead of JOURNAL_ANSWER on the Uplink).
        let consumer = {
            let inbox = std::sync::Arc::clone(&inbox);
            let key = key.clone();
            tokio::spawn(async move {
                inbox
                    .consume_browser_approval("se_rem", Some("rq_rem_1"), &key, false)
                    .await
            })
        };

        // Replay JOURNAL_ANSWER with AllowFor30Min.
        let mut approved_card = pending_card;
        approved_card.state = pb::RequestState::Approved as i32;
        approved_card.answer = Some(pb::RequestAnswer {
            answer: Some(pb::request_answer::Answer::AllowFor30Min(
                pb::AllowFor30Min {},
            )),
        });
        let ans_entry = pb::JournalEntry {
            seq: 2,
            operation: humata2_svc_away::JOURNAL_ANSWER.to_owned(),
            principal: Some(pb::AgentPrincipal {
                session: Some(pb::SessionId {
                    value: "se_rem".to_owned(),
                }),
                pid: 0,
            }),
            request: approved_card.encode_to_vec(),
            ..Default::default()
        };
        assert_eq!(
            inbox.replay("ma_box", &ans_entry).await,
            crate::inbox::replay::Replayed::Applied
        );

        // The racing consumer unblocks and succeeds.
        consumer.await.unwrap().unwrap();

        // Subsequent read with no approval id succeeds under the replayed 30-min grant.
        let read_tab_key = BrowserAction::from_json(r#"{"type":"read_tab"}"#)
            .unwrap()
            .action_key();
        inbox
            .consume_browser_approval("se_rem", None, &read_tab_key, false)
            .await
            .unwrap();

        // A write still fails without per-action approval.
        let click_key = BrowserAction::from_json(&click_json())
            .unwrap()
            .action_key();
        assert!(
            inbox
                .consume_browser_approval("se_rem", None, &click_key, true)
                .await
                .is_err()
        );
    }
}
