//! Each session's agent: how it was launched and how it stands, so a
//! daemon that starts after a crash or reboot can resume it (PER-11,
//! PER-13).

use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};

use prost::Message;
use rusqlite::{Connection, Transaction, params};

use crate::proto::AgentLaunch as LaunchProto;
use crate::store::{Inner, bytes_path, check_id, from_sql, path_bytes, put_tab, to_sql};
use crate::{
    AgentLaunch, AgentRecord, AgentState, Interruption, ResumeFailure, Store, StoreError,
    TabRecord, blob,
};

/// The blob type an agent's launch is kept as.
const LAUNCH_VERSION: u16 = 1;

impl Store {
    /// Saves an agent's record. Its session must be saved.
    pub fn put_agent(&self, agent: &AgentRecord) -> Result<(), StoreError> {
        self.put_agent_with(agent, None)
    }

    /// Saves an agent's record and its tab in one commit, so neither is
    /// ever saved without the other.
    pub fn put_agent_and_tab(&self, agent: &AgentRecord, tab: &TabRecord) -> Result<(), StoreError> {
        check_id(&tab.id)?;
        self.put_agent_with(agent, Some(tab))
    }

    fn put_agent_with(&self, agent: &AgentRecord, tab: Option<&TabRecord>) -> Result<(), StoreError> {
        check_id(&agent.session)?;
        check_id(&agent.tab)?;
        let inner = self.inner();
        let doing = "save a session's agent";
        let encoded = blob::encode(blob::Type::AgentLaunch, LAUNCH_VERSION, &to_proto(&agent.launch).encode_to_vec());
        let op = if tab.is_some() { "agent_and_tab.put" } else { "agent.put" };
        inner.write_for_session(op, doing, Some(&agent.session), |transaction| {
            let placed = blob::place(&inner.blobs, encoded)?;
            (|| {
                if let Some(tab) = tab {
                    put_tab(transaction, tab)?;
                }
                put_row(transaction, agent, placed.inline(), placed.file())
            })()
            .map_err(inner.failed(doing))
        })
    }

    /// The agent of `session`, if one was launched.
    pub fn agent(&self, session: &str) -> Result<Option<AgentRecord>, StoreError> {
        check_id(session)?;
        let inner = self.inner();
        inner.read_with(|connection| {
            let rows = rows(connection, Some(session)).map_err(inner.failed("read an agent"))?;
            rows.into_iter().next().map(|row| row.decode(inner)).transpose()
        })
    }

    /// Every session's agent, oldest session first.
    pub fn agents(&self) -> Result<Vec<AgentRecord>, StoreError> {
        let inner = self.inner();
        inner.read_with(|connection| {
            let rows = rows(connection, None).map_err(inner.failed("list agents"))?;
            rows.into_iter().map(|row| row.decode(inner)).collect()
        })
    }
}

fn put_row(
    transaction: &Transaction<'_>,
    agent: &AgentRecord,
    launch: Option<&[u8]>,
    launch_file: Option<&str>,
) -> rusqlite::Result<()> {
    let (state, holder_pid, exit_code, failure) = match agent.state {
        AgentState::Launching => ("launching", None, None, None),
        AgentState::Running { holder_pid } => ("running", Some(holder_pid), None, None),
        AgentState::Resuming => ("resuming", None, None, None),
        AgentState::Exited { code } => ("exited", None, Some(code), None),
        AgentState::Stopped => ("stopped", None, None, None),
        AgentState::ResumeFailed { reason, exit_code } => {
            ("resume_failed", None, exit_code, Some(failure_name(reason)))
        }
    };
    transaction.execute(
        "INSERT INTO agents (session, tab, state, holder_pid, exit_code, failure, generation,
                             updated_unix_ms, launch, launch_file)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT (session) DO UPDATE SET
             tab = excluded.tab, state = excluded.state, holder_pid = excluded.holder_pid,
             exit_code = excluded.exit_code, failure = excluded.failure,
             generation = excluded.generation, updated_unix_ms = excluded.updated_unix_ms,
             launch = excluded.launch, launch_file = excluded.launch_file",
        params![
            agent.session,
            agent.tab,
            state,
            holder_pid,
            exit_code,
            failure,
            agent.generation,
            agent.updated_unix_ms,
            launch,
            launch_file,
        ],
    )?;
    transaction.execute("DELETE FROM interruptions WHERE session = ?1", [&agent.session])?;
    for (n, interruption) in agent.interruptions.iter().enumerate() {
        transaction.execute(
            "INSERT INTO interruptions (session, n, at_unix_ms, output_saved_through,
                                        output_resumes_at, saved_tail_damaged)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                agent.session,
                n as i64,
                interruption.at_unix_ms,
                to_sql(interruption.output_saved_through),
                to_sql(interruption.output_resumes_at),
                interruption.saved_tail_damaged,
            ],
        )?;
    }
    Ok(())
}

/// An agent's row as read, before its launch is decoded.
struct Row {
    session: String,
    tab: String,
    state: String,
    holder_pid: Option<u32>,
    exit_code: Option<i32>,
    failure: Option<String>,
    generation: u32,
    updated_unix_ms: i64,
    launch: Option<Vec<u8>>,
    launch_file: Option<String>,
    interruptions: Vec<Interruption>,
}

fn rows(connection: &Connection, session: Option<&str>) -> rusqlite::Result<Vec<Row>> {
    let mut statement = connection.prepare(
        "SELECT agents.session, tab, state, holder_pid, exit_code, failure, generation,
                updated_unix_ms, launch, launch_file
         FROM agents JOIN sessions ON sessions.id = agents.session
         WHERE ?1 IS NULL OR agents.session = ?1
         ORDER BY sessions.created_unix_ms, agents.session",
    )?;
    let mut found = statement
        .query_map([session], |row| {
            Ok(Row {
                session: row.get(0)?,
                tab: row.get(1)?,
                state: row.get(2)?,
                holder_pid: row.get(3)?,
                exit_code: row.get(4)?,
                failure: row.get(5)?,
                generation: row.get(6)?,
                updated_unix_ms: row.get(7)?,
                launch: row.get(8)?,
                launch_file: row.get(9)?,
                interruptions: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<Vec<Row>>>()?;
    let mut interruptions = connection.prepare(
        "SELECT at_unix_ms, output_saved_through, output_resumes_at, saved_tail_damaged
         FROM interruptions WHERE session = ?1 ORDER BY n",
    )?;
    for row in &mut found {
        row.interruptions = interruptions
            .query_map([&row.session], |row| {
                Ok(Interruption {
                    at_unix_ms: row.get(0)?,
                    output_saved_through: from_sql(row.get(1)?),
                    output_resumes_at: from_sql(row.get(2)?),
                    saved_tail_damaged: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
    }
    Ok(found)
}

impl Row {
    fn decode(self, inner: &Inner) -> Result<AgentRecord, StoreError> {
        let session = self.session;
        let what = || format!("the agent of session {session}");
        let damaged = |reason| StoreError::Damaged {
            path: inner.database.clone(),
            what: what(),
            reason,
        };
        let state = match self.state.as_str() {
            "launching" => AgentState::Launching,
            "running" => AgentState::Running {
                holder_pid: self.holder_pid.ok_or_else(|| damaged("it runs with no holder"))?,
            },
            "resuming" => AgentState::Resuming,
            "exited" => AgentState::Exited {
                code: self.exit_code.ok_or_else(|| damaged("it exited with no code"))?,
            },
            "stopped" => AgentState::Stopped,
            "resume_failed" => AgentState::ResumeFailed {
                reason: self
                    .failure
                    .as_deref()
                    .and_then(failure_from_name)
                    .ok_or_else(|| damaged("its resume failed for no reason this build knows"))?,
                exit_code: self.exit_code,
            },
            _ => return Err(damaged("its state is one this build doesn't know")),
        };
        let (_, body) = blob::read(
            &inner.blobs,
            &inner.database,
            self.launch,
            self.launch_file,
            blob::Type::AgentLaunch,
            what,
        )?;
        let launch = LaunchProto::decode(body.as_slice())
            .map_err(|_| damaged("how it was launched is unreadable"))?;
        Ok(AgentRecord {
            tab: self.tab,
            launch: from_proto(launch),
            state,
            generation: self.generation,
            interruptions: self.interruptions,
            updated_unix_ms: self.updated_unix_ms,
            session,
        })
    }
}

fn failure_name(reason: ResumeFailure) -> &'static str {
    match reason {
        ResumeFailure::ConversationFileMissing => "conversation_file_missing",
        ResumeFailure::AgentStartedAnotherConversation => "agent_started_another_conversation",
        ResumeFailure::AgentExited => "agent_exited",
        ResumeFailure::AgentNotInstalled => "agent_not_installed",
        ResumeFailure::FolderMissing => "folder_missing",
    }
}

fn failure_from_name(name: &str) -> Option<ResumeFailure> {
    Some(match name {
        "conversation_file_missing" => ResumeFailure::ConversationFileMissing,
        "agent_started_another_conversation" => ResumeFailure::AgentStartedAnotherConversation,
        "agent_exited" => ResumeFailure::AgentExited,
        "agent_not_installed" => ResumeFailure::AgentNotInstalled,
        "folder_missing" => ResumeFailure::FolderMissing,
        _ => return None,
    })
}

fn to_proto(launch: &AgentLaunch) -> LaunchProto {
    let args = |argv: &[OsString]| argv.iter().map(|arg| arg.as_bytes().to_vec()).collect();
    LaunchProto {
        agent: launch.agent.clone(),
        folder: path_bytes(&launch.folder).to_vec(),
        argv: args(&launch.argv),
        resume_argv: args(&launch.resume_argv),
        variable_names: launch.variable_names.clone(),
        conversation_id: launch.conversation_id.clone(),
        conversation_file: path_bytes(&launch.conversation_file).to_vec(),
        holder_socket: path_bytes(&launch.holder_socket).to_vec(),
        cols: u32::from(launch.cols),
        rows: u32::from(launch.rows),
    }
}

fn from_proto(launch: LaunchProto) -> AgentLaunch {
    let args = |argv: Vec<Vec<u8>>| argv.into_iter().map(OsString::from_vec).collect();
    AgentLaunch {
        agent: launch.agent,
        folder: bytes_path(launch.folder),
        argv: args(launch.argv),
        resume_argv: args(launch.resume_argv),
        variable_names: launch.variable_names,
        conversation_id: launch.conversation_id,
        conversation_file: bytes_path(launch.conversation_file),
        holder_socket: bytes_path(launch.holder_socket),
        cols: u16::try_from(launch.cols).unwrap_or(u16::MAX),
        rows: u16::try_from(launch.rows).unwrap_or(u16::MAX),
    }
}

