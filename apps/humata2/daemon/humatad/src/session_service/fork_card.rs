//! What the Inbox card of a fork waiting on Ask me shows (TP-6, IN-10):
//! agent, machine, kind, each repo with how it is copied and its size, and
//! a usage note in words.

use humata2_grpc::humata::v1 as pb;

use super::SessionSvc;
use crate::channel_service::blocking;
use crate::inbox::BoardAsk;

impl SessionSvc {
    /// The card for `request`, or None when its parent can't be read (the
    /// card then shows the capability alone). A parent on another machine
    /// shows its repos with no size, since its clones aren't on this one.
    pub(super) async fn fork_card(&self, request: &pb::ForkSessionRequest) -> Option<BoardAsk> {
        let parent = request.parent.as_ref().map(|parent| parent.value.clone())?;
        let loaded = self.load(&parent).await.ok()?;
        let kind = pb::ForkKind::try_from(request.kind).unwrap_or(pb::ForkKind::Unspecified);
        let copy = if kind == pb::ForkKind::ContextOnly {
            pb::new_session_preview::RepoCopy::ReadsParent
        } else {
            pb::new_session_preview::RepoCopy::BranchAndUncommitted
        };
        let here = self.lives_elsewhere(&loaded.record).is_none();
        let sizes = if here && copy != pb::new_session_preview::RepoCopy::ReadsParent {
            let (starter, id) = (self.starter.clone(), parent.clone());
            // A clone that can't be walked shows no size; the fork itself
            // reports what failed.
            blocking(move || starter.fork_sizes(&id))
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let repos = loaded
            .detail
            .as_ref()
            .map(|detail| {
                detail
                    .repos
                    .iter()
                    .map(|repo| pb::new_session_preview::Repo {
                        repo: Some(pb::RepoRef {
                            owner: repo.owner.clone(),
                            name: repo.name.clone(),
                        }),
                        copy: copy as i32,
                        clone_bytes: sizes
                            .iter()
                            .find(|((owner, name), _)| *owner == repo.owner && *name == repo.name)
                            .map_or(0, |(_, bytes)| *bytes),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let machine = request
            .machine
            .as_ref()
            .map(|machine| machine.value.clone())
            .filter(|machine| !machine.is_empty())
            .or_else(|| (!loaded.record.machine.is_empty()).then(|| loaded.record.machine.clone()))
            .unwrap_or_else(|| self.machine.clone());
        let agent = loaded.record.agent.clone();
        Some(BoardAsk {
            preview: pb::request_preview::Preview::NewSession(pb::NewSessionPreview {
                usage_note: usage_note(&self.starter.agents.person_name(&agent), &agent),
                agent: Some(pb::AgentId { value: agent }),
                model: String::new(),
                machine: Some(pb::MachineId { value: machine }),
                fork_kind: kind as i32,
                repos,
            }),
            reason: request.reason.clone(),
        })
    }
}

/// Whose plan a new agent's usage counts against, in words: Humata has no
/// price feed, so no price is shown (IN-10).
fn usage_note(who: &str, agent: &str) -> String {
    let plan = match agent {
        "claude" => "your Claude plan",
        "codex" => "your Codex plan",
        "copilot" => "your Copilot plan",
        "opencode" => "the provider set up in OpenCode",
        "agy" => "your Antigravity plan",
        _ => "that agent's own account",
    };
    format!("Starts a new {who} agent; its usage counts against {plan}")
}

