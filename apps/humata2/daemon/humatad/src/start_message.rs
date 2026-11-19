//! D763: a start's first message that waits for its agent's input to
//! clear. Meanwhile the session needs you. One still unsent when the wait
//! runs out becomes an Inbox request that carries the whole message:
//! approving it tries to send it again, and the session needs you until it
//! is answered.

use std::sync::{Arc, OnceLock};

use humata2_sessions::{HeldMessages, Resend};
use humata2_store::Store;

use crate::identity::{AgentCaller, Caller};
use crate::inbox::Inbox;
use crate::inbox::permission::{Decision, Question};
use crate::session_service::Asking;
use crate::status;

/// Tells the Inbox and the session's status about start messages that wait.
#[derive(Default)]
pub(crate) struct StartMessages {
    /// Set once the Inbox is made, after the starter.
    told: OnceLock<Told>,
}

/// Where an unsent start message is asked about.
struct Told {
    inbox: Arc<Inbox>,
    store: Arc<Store>,
    /// This daemon's machine.
    machine: String,
}

impl StartMessages {
    pub(crate) fn tell(&self, inbox: Arc<Inbox>, store: Arc<Store>, machine: String) {
        let _ = self.told.set(Told { inbox, store, machine });
    }
}

/// The card's choices.
const SEND: &str = "Send it now";
const LEAVE: &str = "Leave it unsent";

impl HeldMessages for StartMessages {
    fn waiting(&self, session: &str) -> Box<dyn Send> {
        Box::new(Asking::hold(session))
    }

    fn unsent(&self, session: &str, text: &str, resend: Resend) {
        let Some(told) = self.told.get() else {
            crate::log::say!(
                "humatad2: session {session}'s start message wasn't sent and there is no Inbox to say so; type it into the session yourself: {text}"
            );
            return;
        };
        let (inbox, store, machine) = (told.inbox.clone(), told.store.clone(), told.machine.clone());
        // Kept until the request is answered: the session needs you.
        let asking = Asking::hold(session);
        let (session, text) = (session.to_owned(), text.to_owned());
        tokio::spawn(async move {
            let _asking = asking;
            let id = session.clone();
            let found = tokio::task::spawn_blocking(move || store.session(&id)).await;
            let Ok(Ok(Some(record))) = found else {
                crate::log::say!(
                    "humatad2: session {session}'s start message wasn't sent, and the session couldn't be read to say so in the Inbox; type it into the session yourself: {text}"
                );
                return;
            };
            // A question of the session's own: on a machine it is kept in
            // the journal, so the host's Inbox shows it and its answer
            // comes back here (D744).
            let agent = crate::identity::agent_caller_of(&record, &machine);
            let caller = Caller::Agent(agent.clone());
            let card = Question {
                question: unsent_question(&session, &text),
                choices: vec![SEND.to_owned(), LEAVE.to_owned()],
                key: "start-message".to_owned(),
                only_choices: true,
            };
            let mut wait = match inbox.raise_question(&caller, &agent, card).await {
                Ok(wait) => wait,
                Err(error) => {
                    crate::log::say!(
                        "humatad2: couldn't say in the Inbox that session {session}'s start message wasn't sent ({}); type it into the session yourself: {text}",
                        status::chain(&error)
                    );
                    return;
                }
            };
            let decision = wait.decided().await;
            drop(wait);
            if matches!(decision, Decision::Reply { text: picked } if picked == SEND) {
                resend().await;
            }
        });
    }
}

/// What the card asks, with the whole message.
fn unsent_question(session: &str, text: &str) -> String {
    format!(
        "The start message of session {session} wasn't sent: the agent's input didn't clear for it in time, or the agent never showed it. It is here in full:\n\n{text}\n\n\"{SEND}\" types it into the agent once its input is clear and sends it. If the agent's input already holds it, press Enter there instead, so it isn't sent twice. \"{LEAVE}\" or Deny leaves it unsent."
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use humata2_grpc::humata::v1 as pb;
    use humata2_grpc::humata::v1::inbox_service_server::InboxService;
    use humata2_sessions::HeldMessages;
    use humata2_store::{SessionRecord, Store};
    use prost::Message;
    use tokio::sync::oneshot;

    use super::StartMessages;
    use crate::guard::Guard;
    use crate::identity::{Caller, UserVia};
    use crate::inbox::{Inbox, new_inbox};
    use crate::session_service::turns_asking;
    use crate::events::Feed;

    fn user<T>(message: T) -> tonic::Request<T> {
        let mut request = tonic::Request::new(message);
        request.extensions_mut().insert(Caller::User(UserVia::App));
        request
    }

    async fn requests(inbox: &Inbox) -> Vec<pb::InboxRequest> {
        inbox
            .list_requests(user(pb::ListRequestsRequest::default()))
            .await
            .unwrap()
            .into_inner()
            .requests
    }

    fn session(id: &str, machine: &str) -> SessionRecord {
        SessionRecord {
            id: id.to_owned(),
            channel: "ch_a".to_owned(),
            machine: machine.to_owned(),
            title: "Fix the build".to_owned(),
            agent: "copilot".to_owned(),
            folder: std::path::PathBuf::new(),
            created_unix_ms: 0,
        }
    }

    /// The machine's journal, as its link sends it to the host.
    fn journal(store: &Store) -> Vec<pb::JournalEntry> {
        store
            .journal()
            .entries()
            .unwrap()
            .into_iter()
            .map(|kept| {
                let mut entry = pb::JournalEntry::decode(kept.payload.as_slice()).unwrap();
                entry.seq = kept.seq;
                entry
            })
            .collect()
    }

    /// Whether `request` shows `text` whole, for the person to read or send.
    fn carries(request: &pb::InboxRequest, text: &str) -> bool {
        match request.preview.as_ref().and_then(|preview| preview.preview.as_ref()) {
            Some(pb::request_preview::Preview::Typing(typing)) => typing.text == text,
            Some(pb::request_preview::Preview::Question(asked)) => asked.question.contains(text),
            _ => false,
        }
    }

    /// How long a test waits for background work before it fails.
    const PATIENCE: Duration = Duration::from_secs(10);
    /// How often it looks meanwhile.
    const POLL: Duration = Duration::from_millis(2);

    /// Waits for `inbox` to list a request, and returns what it lists.
    async fn listed(inbox: &Inbox) -> Vec<pb::InboxRequest> {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let found = requests(inbox).await;
            if !found.is_empty() || Instant::now() >= deadline {
                return found;
            }
            tokio::time::sleep(POLL).await;
        }
    }

    #[tokio::test]
    async fn a_remote_machines_unsent_start_message_is_asked_about_in_the_hosts_inbox_with_its_text() {
        // The machine the session started on.
        let on_box = tempfile::tempdir().unwrap();
        let box_store = Arc::new(Store::open(on_box.path()).unwrap());
        box_store.put_session(&session("se_remote", "ma_box")).unwrap();
        let box_guard = Arc::new(Guard::new("ma_box", pb::DaemonRole::Machine));
        let box_inbox = new_inbox(
            box_guard,
            box_store.clone(),
            "ma_box".to_owned(),
            Arc::new(Feed::default()),
        );
        let messages = StartMessages::default();
        messages.tell(box_inbox, box_store.clone(), "ma_box".to_owned());
        let text = "reply with the single word pong\nthen stop";
        messages.unsent("se_remote", text, Box::new(|| Box::pin(async {})));

        // The host, which knows the session is on that machine, takes the
        // machine's journal as its link would send it.
        let on_host = tempfile::tempdir().unwrap();
        let host_store = Arc::new(Store::open(on_host.path()).unwrap());
        host_store.put_session(&session("se_remote", "ma_box")).unwrap();
        let host_guard = Arc::new(Guard::new("ma_host", pb::DaemonRole::Host));
        let host = new_inbox(host_guard, host_store, "ma_host".to_owned(), Arc::new(Feed::default()));
        let deadline = Instant::now() + PATIENCE;
        let sent = loop {
            let sent = journal(&box_store);
            if !sent.is_empty() || Instant::now() >= deadline {
                break sent;
            }
            tokio::time::sleep(POLL).await;
        };
        assert!(!sent.is_empty(), "the machine kept nothing for the host to see");
        for entry in &sent {
            host.replay("ma_box", entry).await;
        }

        let shown = listed(&host).await;
        assert_eq!(shown.len(), 1, "the host's Inbox asks about the unsent message");
        assert!(carries(&shown[0], text), "the host's request carries the whole message: {shown:?}");
    }

    #[tokio::test]
    async fn a_waiting_start_message_needs_you_and_an_unsent_one_is_asked_about_with_its_text() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(directory.path()).unwrap());
        let guard = Arc::new(Guard::new("ma_host", pb::DaemonRole::Host));
        store.put_session(&session("se_waits", "ma_host")).unwrap();
        let inbox = new_inbox(guard, store.clone(), "ma_host".to_owned(), Arc::new(Feed::default()));
        let messages = StartMessages::default();
        messages.tell(inbox.clone(), store, "ma_host".to_owned());
        let text = "first line of the start message\nand all of its second line";

        let waiting = messages.waiting("se_waits");
        assert!(turns_asking("se_waits"), "a session whose start message waits needs you");
        drop(waiting);
        assert!(!turns_asking("se_waits"));

        let (resent, heard) = oneshot::channel();
        messages.unsent("se_waits", text, Box::new(move || {
            Box::pin(async move {
                let _ = resent.send(());
            })
        }));
        let mut found = listed(&inbox).await;
        assert_eq!(found.len(), 1, "one request says the start message wasn't sent");
        let request = found.remove(0);
        assert!(carries(&request, text), "the request carries the whole message: {request:?}");
        assert!(turns_asking("se_waits"), "the session needs you while the request waits");

        inbox
            .answer_request(user(pb::AnswerRequestRequest {
                id: request.id.clone(),
                answer: Some(pb::RequestAnswer {
                    answer: Some(pb::request_answer::Answer::Reply(pb::Reply {
                        text: super::SEND.to_owned(),
                    })),
                }),
                ..Default::default()
            }))
            .await
            .unwrap();
        tokio::time::timeout(PATIENCE, heard)
            .await
            .expect("choosing to send it didn't try to send it again")
            .unwrap();
        let deadline = Instant::now() + PATIENCE;
        while turns_asking("se_waits") && Instant::now() < deadline {
            tokio::time::sleep(POLL).await;
        }
        assert!(!turns_asking("se_waits"), "an answered request no longer needs you");
    }
}
