//! D784 "Adopt…": a running holder no session names becomes a session in a
//! channel or the Desk, keeping the holder it has (same pid, same socket,
//! so it still attaches). The session is written as a plain shell session
//! is: its records, then the tab pointed at it. Whatever the orphan runs, it
//! is adopted as a shell-kind session, because an agent's conversation isn't
//! known to us: it keeps running, and a revive would start a shell, not the
//! agent.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_sessions::{Agents, DESK_CHANNEL};
use humata2_store::{
    AgentLaunch, AgentRecord, AgentState, SessionDetail, SessionRecord, StartStep, Store, StoreError,
    TabKind,
};

use super::{RunningError, seen};

/// The plain shell, as Settings › Agents names it.
const SHELL: &str = "zsh";
const COLS: u16 = 80;
const ROWS: u16 = 24;

/// Makes the orphan holder of `tab` a session in `channel` (the Desk when
/// empty) and returns the session's id. Nothing is written unless the tab
/// is a holder no session names, that runs now, into a channel this machine
/// knows (`check_channel` says whether to look for it).
pub(crate) async fn adopt(
    store: &Arc<Store>,
    holders: &Path,
    agents: &Agents,
    home: &Path,
    tab: &str,
    channel: &str,
    check_channel: bool,
) -> Result<String, RunningError> {
    let read = |doing| move |source| RunningError::Store { doing, source };
    let (reader, key) = (store.clone(), tab.to_owned());
    let found = tokio::task::spawn_blocking(move || reader.tab(&key))
        .await
        .map_err(|_| RunningError::Task { doing: "looking up the tab" })?
        .map_err(read("look up the tab"))?;
    let Some(mut record) = found else {
        return Err(RunningError::NoTab { tab: tab.to_owned() });
    };
    if let Some(session) = record.session.clone() {
        return Err(RunningError::AlreadyInSession {
            tab: record.id,
            session,
        });
    }
    let running = seen(holders)
        .await?
        .into_iter()
        .find(|holder| holder.socket == record.holder_socket);
    let Some(running) = running else {
        return Err(RunningError::NotRunning { tab: record.id });
    };
    let channel = if channel.is_empty() { DESK_CHANNEL } else { channel };
    let plan = agents
        .get(SHELL)
        .map_err(|source| RunningError::Shell { source })?;

    let id = format!("se_{}", uuid::Uuid::new_v4().simple());
    let folder = if running.folder.is_empty() {
        let reported = store.tab_program(&record.id).map(|program| program.pwd).unwrap_or_default();
        PathBuf::from(reported)
    } else {
        PathBuf::from(&running.folder)
    };
    let conversation = uuid::Uuid::new_v4().hyphenated().to_string();
    let launch_plan = plan.launch(home, &folder, &conversation);
    let now = unix_ms(SystemTime::now());
    let title = if record.title.is_empty() { running.agent.to_owned() } else { record.title.clone() };
    let session = SessionRecord {
        id: id.clone(),
        channel: channel.to_owned(),
        machine: String::new(),
        title,
        agent: SHELL.to_owned(),
        folder: folder.clone(),
        created_unix_ms: now,
    };
    let detail = SessionDetail {
        session: id.clone(),
        repos: Vec::new(),
        model: None,
        first_message: None,
        start: StartStep::Done,
        operation: None,
        revision: 1,
    };
    let agent = AgentRecord {
        session: id.clone(),
        tab: record.id.clone(),
        launch: AgentLaunch {
            agent: SHELL.to_owned(),
            folder,
            argv: launch_plan.argv,
            resume_argv: launch_plan.resume_argv,
            variable_names: Vec::new(),
            conversation_id: conversation,
            conversation_file: launch_plan.conversation_file,
            holder_socket: record.holder_socket.clone(),
            cols: COLS,
            rows: ROWS,
            cli_version: None,
        },
        state: AgentState::Running { holder_pid: running.pid },
        generation: 1,
        interruptions: Vec::new(),
        updated_unix_ms: now,
    };
    record.session = Some(id.clone());
    record.kind = TabKind::Agent;
    let (store, channel_key) = (store.clone(), channel.to_owned());
    tokio::task::spawn_blocking(move || -> Result<(), RunningError> {
        let saving = |doing| move |source: StoreError| RunningError::Store { doing, source };
        if check_channel && channel_key != DESK_CHANNEL {
            match store.channel(&channel_key) {
                Ok(Some(_)) => {}
                Ok(None) | Err(StoreError::InvalidId { .. }) => {
                    return Err(RunningError::NoChannel { channel: channel_key });
                }
                Err(source) => return Err(saving("look up the channel")(source)),
            }
        }
        store.put_session(&session).map_err(saving("save the adopted session"))?;
        store.put_session_detail(&detail).map_err(saving("save the adopted session's start"))?;
        store.put_agent(&agent).map_err(saving("save the adopted session's agent"))?;
        store.put_tab(&record).map_err(saving("point the tab at its new session"))
    })
    .await
    .map_err(|_| RunningError::Task { doing: "adopting the holder" })??;
    Ok(id)
}

fn unix_ms(at: SystemTime) -> i64 {
    at.duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}
