//! Planning with the agent: the tasks and spec edits a session's agent
//! proposes, read back from its history, where they are held on its host;
//! the person's decision on each; the specs a session reads, copied to its
//! host; and telling sessions when a spec they read changes.

use std::collections::HashMap;

use humata_wire::{Reply, pb};

use super::Daemon;
use crate::board::{Author, Item, ItemFields, ItemPatch, Priority, Status};
use crate::error::DaemonError;
use crate::store::{Decision, Project};

impl Daemon {
    /// `session`'s proposals, oldest first, with what the person did about
    /// each.
    pub(super) async fn proposals(&self, session: &str) -> Result<Vec<pb::Proposal>, DaemonError> {
        let host = self.host_of(session).await?;
        let read = pb::ReadHistory {
            session_id: session.to_owned(),
        };
        let events = match self.history(&host, read).await? {
            Reply::Response(pb::response::Kind::History(history)) => history.events,
            _ => Vec::new(),
        };
        self.decided(session, proposed(&events, session))
    }

    /// What outside agents proposed for `project`'s board, oldest first,
    /// with what the person did about each.
    pub(super) fn outside_proposals(
        &self,
        project: &str,
    ) -> Result<Vec<pb::Proposal>, DaemonError> {
        self.store.project(project)?;
        self.decided("", self.store.outside_proposals(project)?)
    }

    /// Keeps what an outside agent proposed, through `humata mcp`, until the
    /// person decides it in the app.
    pub(super) fn propose(&self, propose: pb::Propose) -> Result<pb::Proposal, DaemonError> {
        self.store.project(&propose.project_id)?;
        let in_project = |id: &str| -> Result<(), DaemonError> {
            let item = self.store.item(id)?;
            if item.project_id == propose.project_id {
                Ok(())
            } else {
                Err(crate::error::StoreError::UnknownItem { id: id.to_owned() }.into())
            }
        };
        let blank = |text: &str| text.trim().is_empty();
        match propose.kind() {
            pb::ProposalKind::SpecEdit => {
                if blank(&propose.item_id) || (blank(&propose.title) && blank(&propose.spec)) {
                    return Err(DaemonError::IncompleteProposal);
                }
                in_project(&propose.item_id)?;
            }
            _ => {
                if blank(&propose.title) {
                    return Err(DaemonError::IncompleteProposal);
                }
                if !propose.parent_id.is_empty() {
                    in_project(&propose.parent_id)?;
                }
            }
        }
        let kind = match propose.kind() {
            pb::ProposalKind::SpecEdit => pb::ProposalKind::SpecEdit,
            _ => pb::ProposalKind::Task,
        };
        let proposal = pb::Proposal {
            id: uuid::Uuid::new_v4().to_string(),
            kind: kind.into(),
            title: propose.title,
            spec: propose.spec,
            parent_id: propose.parent_id,
            item_id: propose.item_id,
            note: propose.note,
            project_id: propose.project_id,
            time_unix_ms: crate::store::now_unix_ms(),
            ..pb::Proposal::default()
        };
        self.store.add_outside_proposal(&proposal)?;
        let mut decided = self.decided("", vec![proposal])?;
        Ok(decided.remove(0))
    }

    /// `proposals` of `session` (empty for outside agents'), with what the
    /// person did about each.
    fn decided(
        &self,
        session: &str,
        mut proposals: Vec<pb::Proposal>,
    ) -> Result<Vec<pb::Proposal>, DaemonError> {
        let decisions: HashMap<String, Decision> =
            self.store.decisions(session)?.into_iter().collect();
        for proposal in &mut proposals {
            match decisions.get(&proposal.id) {
                Some(Decision::Applied { item }) => {
                    proposal.state = pb::ProposalState::Applied.into();
                    proposal.applied_item_id.clone_from(item);
                }
                Some(Decision::Declined) => {
                    proposal.state = pb::ProposalState::Declined.into();
                }
                None => proposal.state = pb::ProposalState::Pending.into(),
            }
            // A spec edit shows the item's title when it proposes none, and
            // while pending the spec it would replace.
            if proposal.kind() == pb::ProposalKind::SpecEdit
                && let Ok(item) = self.store.item(&proposal.item_id)
            {
                if proposal.state() == pb::ProposalState::Pending {
                    proposal.current_spec.clone_from(&item.fields.spec);
                }
                if proposal.title.is_empty() {
                    proposal.title = item.fields.title;
                }
            }
        }
        Ok(proposals)
    }

    /// The pending proposal `id` of `session`, or of an outside agent if
    /// `session` is empty.
    pub(super) async fn pending(
        &self,
        session: &str,
        id: &str,
    ) -> Result<pb::Proposal, DaemonError> {
        let unknown = || DaemonError::UnknownProposal {
            session: session.to_owned(),
            proposal: id.to_owned(),
        };
        let proposals = if session.is_empty() {
            let proposal = self.store.outside_proposal(id)?.ok_or_else(unknown)?;
            self.decided("", vec![proposal])?
        } else {
            self.proposals(session).await?
        };
        let proposal = proposals
            .into_iter()
            .find(|proposal| proposal.id == id)
            .ok_or_else(unknown)?;
        if proposal.state() != pb::ProposalState::Pending {
            return Err(crate::error::StoreError::AlreadyDecided {
                proposal: id.to_owned(),
            }
            .into());
        }
        Ok(proposal)
    }

    /// Adds the proposed task, or changes the item's spec, as proposed or
    /// as edited in `apply`.
    pub(super) async fn apply_proposal(
        &self,
        apply: pb::ApplyProposal,
    ) -> Result<pb::ProposalApplied, DaemonError> {
        let mut proposal = self.pending(&apply.session_id, &apply.proposal_id).await?;
        let (project, author) = if apply.session_id.is_empty() {
            // An outside agent's: the board's log says it came through MCP.
            (
                proposal.project_id.clone(),
                Author::on(pb::ChangeSurface::Mcp),
            )
        } else {
            let project = self
                .store
                .placement(&apply.session_id)?
                .and_then(|placement| placement.project)
                .ok_or_else(|| DaemonError::NoBoard {
                    session: apply.session_id.clone(),
                })?;
            (
                project,
                Author::session(&apply.session_id, pb::ChangeSurface::Unspecified),
            )
        };
        let item = match proposal.kind() {
            pb::ProposalKind::SpecEdit => {
                // A proposal may change only the title, or only the spec.
                let patch = ItemPatch {
                    status: None,
                    priority: None,
                    title: apply
                        .title
                        .or_else(|| Some(proposal.title.clone()))
                        .filter(|title| !title.trim().is_empty()),
                    spec: apply
                        .spec
                        .or_else(|| Some(proposal.spec.clone()))
                        .filter(|spec| !spec.trim().is_empty()),
                };
                let before = self.store.item(&proposal.item_id).ok();
                let item = self
                    .store
                    .update_item(&proposal.item_id, patch, &author)
                    .map_err(|_| DaemonError::NoProposedItem {
                        proposal: proposal.id.clone(),
                    })?;
                if before.is_none_or(|before| before.fields.spec != item.fields.spec) {
                    self.spec_changed(&item, "").await;
                }
                item
            }
            _ => {
                let fields = ItemFields {
                    parent_id: crate::convert::optional(proposal.parent_id.clone()),
                    status: Status::Todo,
                    priority: Priority::None,
                    title: apply.title.unwrap_or_else(|| proposal.title.clone()),
                    spec: apply.spec.unwrap_or_else(|| proposal.spec.clone()),
                };
                self.store.add_item(&project, fields, &author)?
            }
        };
        self.store.decide(
            &apply.session_id,
            &proposal.id,
            &Decision::Applied {
                item: item.id.clone(),
            },
        )?;
        proposal.state = pb::ProposalState::Applied.into();
        proposal.applied_item_id.clone_from(&item.id);
        let item = self.shown_item(&item).await?;
        Ok(pb::ProposalApplied {
            proposal: Some(proposal),
            item: Some(item),
        })
    }

    pub(super) async fn decline_proposal(
        &self,
        decline: &pb::DeclineProposal,
    ) -> Result<pb::Proposal, DaemonError> {
        let mut proposal = self
            .pending(&decline.session_id, &decline.proposal_id)
            .await?;
        self.store
            .decide(&decline.session_id, &proposal.id, &Decision::Declined)?;
        proposal.state = pb::ProposalState::Declined.into();
        Ok(proposal)
    }

    pub(super) async fn update_project(
        &self,
        update: pb::UpdateProject,
    ) -> Result<pb::Project, DaemonError> {
        // Cloned first, so a URL that can't be cloned changes nothing.
        let repositories = match update.repositories {
            Some(wanted) => Some(self.resolve_repositories(wanted.repositories).await?),
            None => None,
        };
        self.store.update_project(
            &update.project_id,
            update.name.as_deref().map(str::trim),
            update.instructions.as_deref(),
        )?;
        if let Some(skills) = update.skills {
            self.store
                .set_skills(&update.project_id, &crate::convert::skills(skills.skills))?;
        }
        if let Some((repositories, urls)) = repositories {
            self.store
                .set_repositories(&update.project_id, &repositories, &urls)?;
        }
        Ok(crate::convert::project(
            self.store.project(&update.project_id)?,
        ))
    }

    /// Sets up `fork` to work on `item`, reading its specs: a fork of a
    /// planning session also gets its own copy of the project's
    /// repositories. Returns the item's project.
    pub(super) async fn fork_onto(
        &self,
        fork: &mut pb::ForkSession,
        host: &str,
        item: &str,
        project: Option<&str>,
    ) -> Result<String, DaemonError> {
        let item = self.store.item(item)?;
        if project.is_some_and(|project| project != item.project_id) {
            return Err(crate::error::StoreError::ItemInAnotherProject {
                title: item.fields.title,
            }
            .into());
        }
        let project = self.store.project(&item.project_id)?;
        if self.store.planning(&fork.session_id)? {
            fork.chat_only = false;
            fork.repositories = if host.is_empty() {
                project
                    .repositories
                    .iter()
                    .map(crate::convert::wire_repository)
                    .collect()
            } else {
                self.clone_on(host, &project).await?.repositories
            };
        }
        let parent = match &item.fields.parent_id {
            Some(parent) => Some(self.store.item(parent)?),
            None => None,
        };
        fork.instructions = Some(instructions(&project, "", Some(&item), parent.as_ref()));
        Ok(project.id)
    }

    /// The specs `session` reads: its item's, and its task's for a subtask.
    fn specs_of(&self, session: &str) -> Result<Vec<pb::Spec>, DaemonError> {
        let Some(item) = self
            .store
            .placement(session)?
            .and_then(|placement| placement.item)
        else {
            return Ok(Vec::new());
        };
        let item = self.store.item(&item)?;
        let mut specs = Vec::new();
        if let Some(parent) = &item.fields.parent_id {
            specs.push(spec(&self.store.item(parent)?));
        }
        specs.push(spec(&item));
        Ok(specs)
    }

    /// Copies the specs `session` reads to its host, so its agent reads them
    /// while the Mac is away. Sessions here read them from the daemon.
    pub(super) async fn push_specs(&self, session: &str, host: &str) -> Result<(), DaemonError> {
        if host.is_empty() {
            return Ok(());
        }
        let set = pb::request::Kind::SetSpecs(pb::SetSpecs {
            session_id: session.to_owned(),
            specs: self.specs_of(session)?,
        });
        self.ask(host, set).await?;
        Ok(())
    }

    /// The spec `item` of `session`'s board.
    pub(super) fn read_spec(&self, read: &pb::ReadSpec) -> Result<pb::Spec, DaemonError> {
        let item = self.store.item(&read.item_id)?;
        let project = self
            .store
            .placement(&read.session_id)?
            .and_then(|placement| placement.project);
        if project.as_deref() != Some(item.project_id.as_str()) {
            return Err(crate::error::StoreError::UnknownItem {
                id: read.item_id.clone(),
            }
            .into());
        }
        Ok(spec(&item))
    }

    /// Tells the sessions that read `item`'s spec that it changed, except
    /// `by`, whose agent made the change. Each gets the new copy on its
    /// host. A running or waiting agent is asked to read it again before it
    /// goes on; any other is told with the person's next message, so the
    /// change starts no turn. Archived sessions aren't told.
    pub(super) async fn spec_changed(&self, item: &Item, by: &str) {
        let Ok(sessions) = self.store.sessions_on(&item.id) else {
            return;
        };
        if sessions.is_empty() {
            return;
        }
        let listed = self
            .list("")
            .await
            .map(|listed| listed.sessions)
            .unwrap_or_default();
        for (session, host) in sessions {
            let _ = self.push_specs(&session, &host).await;
            let info = listed.iter().find(|info| info.id == session);
            if session == by || info.is_some_and(|info| info.archived) {
                continue;
            }
            let busy = info.is_some_and(|info| {
                !info.host_offline
                    && matches!(
                        info.state(),
                        pb::SessionState::Working | pb::SessionState::WaitingForApproval
                    )
            });
            let text = format!(
                "The spec of {:?} changed. Read it again with read_item (id {}) before you go on, and say if it changes your plan.",
                item.fields.title, item.id
            );
            if busy {
                let message = pb::request::Kind::SendMessage(pb::SendMessage {
                    session_id: session.clone(),
                    text: text.clone(),
                    mode: pb::SendMode::Enqueue.into(),
                });
                // One the host wouldn't queue hears of it with its next
                // message instead.
                if self
                    .forward(&host, message)
                    .await
                    .is_ok_and(|reply| super::refusal(&reply).is_none())
                {
                    continue;
                }
            }
            if let Err(error) = self.store.note_spec(&session, &item.id, &text) {
                eprintln!("humatad: couldn't keep the spec note for session {session}: {error}");
            }
        }
    }
}

fn spec(item: &Item) -> pb::Spec {
    pb::Spec {
        item_id: item.id.clone(),
        title: item.fields.title.clone(),
        spec: item.fields.spec.clone(),
        parent_id: item.fields.parent_id.clone().unwrap_or_default(),
    }
}

/// What a session's instructions are made of, in order: its project's
/// instructions, its own, its project's skills, then the specs it reads,
/// its task's before its subtask's.
pub(super) fn instructions(
    project: &Project,
    own: &str,
    item: Option<&Item>,
    parent: Option<&Item>,
) -> String {
    let mut text = String::new();
    let mut section = |part: &str| {
        let part = part.trim();
        if !part.is_empty() {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(part);
        }
    };
    section(&project.instructions);
    if own.trim() != project.instructions.trim() {
        section(own);
    }
    for skill in &project.skills {
        section(&format!("# Skill: {}\n\n{}", skill.name, skill.text.trim()));
    }
    if let Some(parent) = parent {
        section(&spec_section("The task this is part of", parent));
    }
    if let Some(item) = item {
        let heading = if item.fields.parent_id.is_some() {
            "Your subtask"
        } else {
            "Your task"
        };
        section(&spec_section(heading, item));
    }
    text
}

fn spec_section(heading: &str, item: &Item) -> String {
    format!(
        "# {heading}: {}\n\n{}",
        item.fields.title,
        item.fields.spec.trim()
    )
}

/// The proposals in `events`: calls to `propose_task` and
/// `propose_spec_edit`, by whatever name the agent's CLI gives them, that
/// succeeded.
fn proposed(events: &[pb::SessionEvent], session: &str) -> Vec<pb::Proposal> {
    use pb::session_event::Kind;
    let mut proposals: Vec<(pb::Proposal, bool)> = Vec::new();
    let mut turn = 0;
    for event in events {
        match &event.kind {
            Some(Kind::TurnStarted(_)) => turn += 1,
            Some(Kind::ToolStarted(call)) => {
                if let Some(mut proposal) = proposal(call) {
                    proposal.session_id = session.to_owned();
                    proposal.turn = turn.max(1);
                    proposal.time_unix_ms = event.time_unix_ms;
                    proposals.push((proposal, false));
                }
            }
            Some(Kind::ToolCompleted(done)) => {
                for (proposal, succeeded) in &mut proposals {
                    if proposal.id == done.call_id {
                        *succeeded = done.success;
                    }
                }
            }
            _ => {}
        }
    }
    proposals
        .into_iter()
        .filter(|(_, succeeded)| *succeeded)
        .map(|(proposal, _)| proposal)
        .collect()
}

fn proposal(call: &pb::ToolStarted) -> Option<pb::Proposal> {
    let kind = if call.tool.ends_with("propose_task") {
        pb::ProposalKind::Task
    } else if call.tool.ends_with("propose_spec_edit") {
        pb::ProposalKind::SpecEdit
    } else {
        return None;
    };
    let arguments: serde_json::Value = serde_json::from_str(&call.arguments_json).ok()?;
    let text = |field: &str| {
        arguments[field]
            .as_str()
            .map(str::trim)
            .unwrap_or_default()
            .to_owned()
    };
    let proposal = pb::Proposal {
        id: call.call_id.clone(),
        kind: kind.into(),
        title: text("title"),
        spec: arguments["spec"].as_str().unwrap_or_default().to_owned(),
        parent_id: text("parent_id"),
        item_id: text("item_id"),
        note: text("note"),
        ..Default::default()
    };
    let complete = match kind {
        pb::ProposalKind::SpecEdit => !proposal.item_id.is_empty(),
        _ => !proposal.title.is_empty(),
    };
    complete.then_some(proposal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Skill;

    fn item(title: &str, parent: Option<&str>, spec: &str) -> Item {
        Item {
            id: title.to_owned(),
            project_id: "p".to_owned(),
            fields: ItemFields {
                parent_id: parent.map(str::to_owned),
                title: title.to_owned(),
                spec: spec.to_owned(),
                status: Status::Todo,
                priority: Priority::None,
            },
            created_unix_ms: 0,
            updated_unix_ms: 0,
            session_ids: Vec::new(),
        }
    }

    #[test]
    fn a_session_reads_its_projects_instructions_its_own_skills_then_specs_parents_first() {
        let project = Project {
            id: "p".into(),
            name: "p".into(),
            repositories: Vec::new(),
            urls: std::collections::BTreeMap::new(),
            instructions: "Use tabs.".into(),
            created_unix_ms: 0,
            skills: vec![Skill {
                name: "release".into(),
                text: "Tag it.".into(),
            }],
        };
        let task = item("login", None, "Let people sign in.");
        let subtask = item("form", Some("login"), "The form.");
        let text = instructions(&project, "Be brief.", Some(&subtask), Some(&task));
        let at = |part: &str| text.find(part).unwrap_or(usize::MAX);
        let order = [
            at("Use tabs."),
            at("Be brief."),
            at("Tag it."),
            at("Let people sign in."),
            at("The form."),
        ];
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
        assert!(order.iter().all(|at| *at != usize::MAX), "{text}");

        let alone = instructions(&project, "", Some(&subtask), None);
        assert!(!alone.contains("Let people sign in."), "{alone}");
    }

    fn event(kind: pb::session_event::Kind) -> pb::SessionEvent {
        pb::SessionEvent {
            kind: Some(kind),
            ..Default::default()
        }
    }

    fn call(id: &str, tool: &str, arguments: &str) -> pb::SessionEvent {
        event(pb::session_event::Kind::ToolStarted(pb::ToolStarted {
            call_id: id.into(),
            tool: tool.into(),
            arguments_json: arguments.into(),
        }))
    }

    fn done(id: &str, success: bool) -> pb::SessionEvent {
        event(pb::session_event::Kind::ToolCompleted(pb::ToolCompleted {
            call_id: id.into(),
            success,
            output: String::new(),
        }))
    }

    #[test]
    fn only_proposals_that_succeeded_count_whatever_the_tool_is_called() {
        let events = [
            event(pb::session_event::Kind::TurnStarted(pb::TurnStarted {})),
            call(
                "a",
                "mcp__humata__propose_task",
                r#"{"title": "Log in", "spec": "With email."}"#,
            ),
            done("a", true),
            call("b", "propose_task", r#"{"title": "Failed"}"#),
            done("b", false),
            call(
                "c",
                "propose_spec_edit",
                r#"{"item_id": "i1", "spec": "New."}"#,
            ),
            done("c", true),
            call("d", "propose_task", r#"{"spec": "no title"}"#),
            done("d", true),
            call("e", "board", "{}"),
            done("e", true),
        ];
        let found = proposed(&events, "s");
        let ids: Vec<&str> = found.iter().map(|proposal| proposal.id.as_str()).collect();
        assert_eq!(ids, ["a", "c"]);
        assert_eq!(found[0].kind(), pb::ProposalKind::Task);
        assert_eq!(found[0].spec, "With email.");
        assert_eq!(found[1].kind(), pb::ProposalKind::SpecEdit);
        assert_eq!(found[1].item_id, "i1");
        assert_eq!(found[1].turn, 1);
    }
}
