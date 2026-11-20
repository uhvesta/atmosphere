//! Continue in another channel (CV-8, D307): a new top-level session in the
//! chosen channel, set up as that channel's New session (its ready
//! repositories, a branch from its main clones), whose agent resumes a copy
//! of another session's conversation, from now or from before a turn
//! (CV-2), under a new id and placed for its new folder as a fork's is
//! (MV-16). The other session is untouched.
//!
//! The records and the note naming the copy are saved together, so a cut
//! from there until the agent starts leaves a session that a restart takes
//! back (`recover`), its copy and folder with it.

use std::fs;
use std::path::Path;

use humata2_revive::Env;
use humata2_store::fault::point;
use humata2_store::{ChannelRecord, SessionDetail, SessionRecord, SessionRepo, StartStep};

use crate::error::StartError;
use crate::fork::{Through, carry_conversation, copy_note, forget_note, note_copy_at};
use crate::setup::unix_ms;
use crate::start::{Started, Starter, blocking, store_err};

/// A session's conversation to continue in another channel.
#[derive(Clone, Debug)]
pub struct ContinueRequest {
    /// The new session's id; None: a fresh one.
    pub id: Option<String>,
    /// The session whose conversation goes on.
    pub from: String,
    /// The channel the new session starts in.
    pub channel: String,
    /// The machine the new session runs on.
    pub machine: String,
    /// Up to this turn; 0: the whole conversation.
    pub before_turn: u32,
    pub operation: Option<String>,
    pub cols: u16,
    pub rows: u16,
}

impl Starter {
    /// Starts the new session of `request` and its agent on the copy.
    /// `progress` hears `write_record`, `clone:<repo>`, `copy_conversation`,
    /// `launch` and `verify`.
    pub async fn continue_in(
        &self,
        request: ContinueRequest,
        env: Env<'_>,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<Started, StartError> {
        let this = self.clone();
        let (from_id, channel_id) = (request.from.clone(), request.channel.clone());
        let (from, from_detail, from_agent, channel) = blocking(move || {
            let (record, detail) = this.parent(&from_id)?;
            let agent = this
                .store
                .agent(&from_id)
                .map_err(store_err("read the session's agent"))?;
            let channel = this.channel_record(&channel_id)?;
            Ok::<_, StartError>((record, detail, agent, channel))
        })
        .await??;
        let from_agent = from_agent.ok_or_else(|| StartError::NoAgent {
            session: from.id.clone(),
        })?;
        let chosen = self.choose(&channel, None)?;
        let this = self.clone();
        let (picked, repos, title) = (channel.clone(), chosen.clone(), from.title.clone());
        let branch = blocking(move || this.pick(&picked, &repos, &title)).await??;
        let folder = self.layout.session(&channel.folder, &branch.folder);
        let conversation = uuid::Uuid::new_v4().hyphenated().to_string();
        let id = request
            .id
            .clone()
            .unwrap_or_else(|| format!("se_{}", uuid::Uuid::new_v4().simple()));
        // The source is read where its session runs it; the copy lands in the
        // new session's own config home, where its launch records it.
        let from_kind = self.agents.get_as_ran(&from.id, &from.agent)?;
        let kind = self.agents.get_for_session(&id, &from.agent)?;
        let to = kind.launch(&self.home, &folder, &conversation).conversation_file;
        let through = match (
            from_kind.placement(&self.home, &from.folder),
            kind.placement(&self.home, &folder),
        ) {
            (Some(from_at), Some(to_at)) => Some(Through {
                format: kind.conversation_format,
                from: from_at,
                from_id: from_agent.launch.conversation_id.clone(),
                to: to_at,
                to_id: conversation.clone(),
            }),
            _ => None,
        };
        let session = SessionRecord {
            id,
            channel: channel.id.clone(),
            machine: request.machine.clone(),
            title: from.title.clone(),
            agent: from.agent.clone(),
            folder: folder.clone(),
            created_unix_ms: unix_ms(),
        };
        let mut detail = SessionDetail {
            session: session.id.clone(),
            repos: chosen
                .iter()
                .map(|(owner, name)| SessionRepo {
                    owner: owner.clone(),
                    name: name.clone(),
                    branch: branch.name.clone(),
                    path: folder.join(name),
                })
                .collect(),
            model: from_detail.model.clone(),
            first_message: None,
            start: StartStep::Recorded,
            operation: request.operation.clone(),
            revision: 1,
        };

        let copy = Copy {
            from: &from_agent.launch.conversation_file,
            to: &to,
            through,
            before_turn: request.before_turn,
        };
        let early = self
            .record_and_carry(&channel, &session, &mut detail, copy, progress)
            .await;
        if let Err(error) = early {
            // Best effort: the continue's own error is the one worth reporting.
            let _ = self.undo(&session).await;
            let _ = fs::remove_file(&to);
            return Err(error);
        }
        let (detail, agent) = self
            .launch_with(
                &session,
                detail,
                request.cols,
                request.rows,
                env,
                progress,
                conversation,
                true,
            )
            .await?;
        // The agent record names the copy now.
        let (folder, id) = (session.folder.clone(), session.id.clone());
        let _ = blocking(move || forget_note(&copy_note(&folder, &id))).await;
        Ok(Started {
            session,
            detail,
            agent,
        })
    }

    /// Saves the new session's records with the note naming its copy, clones
    /// its repositories from the channel's main clones, then copies the
    /// conversation.
    async fn record_and_carry(
        &self,
        channel: &ChannelRecord,
        session: &SessionRecord,
        detail: &mut SessionDetail,
        copy: Copy<'_>,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<(), StartError> {
        let (store, saved, saved_detail) = (self.store.clone(), session.clone(), detail.clone());
        let (note, noted) = (copy_note(&session.folder, &session.id), copy.to.to_owned());
        blocking(move || -> Result<(), StartError> {
            // The note first: a restart that finds the records finds it too,
            // so it takes back a continue cut short rather than starting it
            // plainly, and a folder holding only the note is taken back alone.
            note_copy_at(&note, &noted).map_err(|source| StartError::Copy {
                path: note.clone(),
                source,
            })?;
            point("sessions.continue.noted");
            store
                .put_session_with_detail(&saved, &saved_detail)
                .map_err(store_err("save the session and its start"))
        })
        .await??;
        point("sessions.continue.recorded");
        progress("write_record");
        self.clone_repos(channel, detail, progress).await?;
        self.set_step(detail, StartStep::Cloned).await?;
        let (folder, id) = (session.folder.clone(), session.id.clone());
        let (from, to, through, before_turn) =
            (copy.from.to_owned(), copy.to.to_owned(), copy.through, copy.before_turn);
        blocking(move || carry_conversation(&folder, &id, &from, &to, through, before_turn))
            .await??;
        point("sessions.continue.copied");
        progress("copy_conversation");
        Ok(())
    }
}

/// The conversation a continued session carries: where it is, where its
/// copy goes, how its adapter finds them, and up to which turn.
struct Copy<'a> {
    from: &'a Path,
    to: &'a Path,
    through: Option<Through>,
    before_turn: u32,
}
