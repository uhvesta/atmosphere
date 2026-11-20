//! Terminal tabs: which tab is which index, and the links a tab's view
//! and `write` go through.

use std::sync::Arc;

use humata2_clients::model::{Agent, Session, SessionId};
use humata2_clients::{
    DaemonError, Event, Events, ImageKind, Reply, TerminalLink, TerminalNotice, Terminals,
};
use crate::humata2_grpc::humata::v1 as pb;
use terminal_wire::{ClientFrame, ServerFrame};

use crate::connection::{LiveDaemon, Shared, fresh_key};
use crate::map;

/// How many times a tab not listed yet is looked for, and how long apart:
/// about two seconds in all.
const TAB_TRIES: u32 = 20;
const TAB_RETRY: std::time::Duration = std::time::Duration::from_millis(100);

/// The longest a session still starting is waited for to list its first
/// tab: that tab is made once its program launches, after the worktree and
/// the holder.
const START_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

fn no_tab(index: usize) -> DaemonError {
    DaemonError::Refused {
        what: "The terminal".to_owned(),
        reason: format!(
            "the session has no terminal tab {index}. Open the tab again from the session"
        ),
    }
}

fn image_format(kind: ImageKind) -> pb::ImageFormat {
    match kind {
        ImageKind::Png => pb::ImageFormat::Png,
        ImageKind::Jpeg => pb::ImageFormat::Jpeg,
        ImageKind::Gif => pb::ImageFormat::Gif,
        ImageKind::Webp => pb::ImageFormat::Webp,
        ImageKind::Tiff => pb::ImageFormat::Tiff,
        ImageKind::Heic => pb::ImageFormat::Heic,
    }
}

fn pb_session(id: &SessionId) -> pb::SessionId {
    pb::SessionId {
        value: id.as_str().to_owned(),
    }
}

impl Shared {
    /// The session's tabs, read from the daemon and remembered.
    pub(crate) async fn refresh_tabs(
        &self,
        session: &pb::SessionId,
    ) -> Result<Vec<pb::Tab>, DaemonError> {
        let tabs = self
            .terminal()
            .list_tabs(pb::ListTabsRequest {
                session: Some(session.clone()),
            })
            .await
            .map_err(|status| self.error(&status, "The session's terminals"))?
            .into_inner()
            .tabs;
        let tabs = map::order_tabs(tabs);
        self.state().tabs.insert(session.value.clone(), tabs.clone());
        Ok(tabs)
    }

    /// The session's tabs listed now, or, when the daemon can't list them,
    /// the last list it gave: a session shown with no tabs would close the
    /// ones open.
    pub(crate) async fn listed_tabs(&self, session: &pb::SessionId) -> Vec<pb::Tab> {
        match self.refresh_tabs(session).await {
            Ok(tabs) => tabs,
            Err(_) => self.state().tabs.get(&session.value).cloned().unwrap_or_default(),
        }
    }

    /// The session as the app shows it.
    pub(crate) async fn fetch_session(self: &Arc<Self>, id: &pb::SessionId) -> Result<Session, DaemonError> {
        let session = self
            .sessions()
            .get_session(pb::GetSessionRequest {
                id: Some(id.clone()),
            })
            .await
            .map_err(|status| self.error(&status, "The session"))?
            .into_inner();
        let tabs = self.listed_tabs(id).await;
        Ok(self.map_session(&session, &tabs).await)
    }

    /// Tab `index` of the session: 0 is the agent's.
    pub(crate) async fn tab(
        self: &Arc<Self>,
        session: &SessionId,
        index: usize,
    ) -> Result<pb::Tab, DaemonError> {
        if let Some(tab) = self.cached_tab(session, index) {
            return Ok(tab);
        }
        // A session still starting gets its first tab when its program
        // launches. Listening before looking means the push that brings it
        // can't fall between the look and the wait.
        let events = if index == 0 { self.subscribe().await.ok() } else { None };
        if let Some(events) = events {
            if let Some(tab) = self.refresh_tabs(&pb_session(session)).await?.into_iter().next() {
                return Ok(tab);
            }
            let shared = self.clone();
            let id = session.clone();
            // On the client's runtime: the caller may be polled elsewhere,
            // and a tokio timer needs the runtime it is made on.
            let waited = self
                .spawn(async move {
                    tokio::time::timeout(START_WAIT, shared.first_tab(&id, events)).await
                })
                .await;
            return match waited {
                Ok(Ok(found)) => found,
                Ok(Err(_)) => Err(DaemonError::Refused {
                    what: "The terminal".to_owned(),
                    reason: format!(
                        "the session was still starting after {} seconds, so it has no terminal yet. Check its machine in the session's Info, then open the tab again",
                        START_WAIT.as_secs()
                    ),
                }),
                Err(_) => Err(no_tab(index)),
            };
        }
        // With no event stream: a session just started lists its tabs a moment after the app
        // hears of it, so a tab not listed yet gets a few short tries
        // before it is refused.
        for _ in 0..TAB_TRIES {
            if let Some(tab) = self
                .refresh_tabs(&pb_session(session))
                .await?
                .into_iter()
                .nth(index)
            {
                return Ok(tab);
            }
            // On the client's runtime: the caller may be polled elsewhere,
            // and a tokio sleep needs the runtime from when it is made, so
            // it is made inside the spawned task.
            let _ = self.spawn(async { tokio::time::sleep(TAB_RETRY).await }).await;
        }
        Err(no_tab(index))
    }

    /// Tab `index` as the client last listed it.
    fn cached_tab(&self, session: &SessionId, index: usize) -> Option<pb::Tab> {
        self.state()
            .tabs
            .get(session.as_str())
            .and_then(|tabs| tabs.get(index).cloned())
    }

    /// The session's first tab, once its start ends. Each push about the
    /// session (its tabs are listed again before it is published) is a
    /// chance to find it; a start that ended with no tab is refused.
    async fn first_tab(&self, session: &SessionId, mut events: Events) -> Result<pb::Tab, DaemonError> {
        loop {
            if !self.starting(session).await? {
                return self
                    .refresh_tabs(&pb_session(session))
                    .await?
                    .into_iter()
                    .next()
                    .ok_or_else(|| no_tab(0));
            }
            loop {
                match events.recv().await {
                    Some(Ok(Event::Session(changed))) if &changed.id == session => break,
                    Some(Ok(Event::Resync)) => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => return Err(error),
                    None => {
                        return Err(DaemonError::Offline {
                            machine: self.machine.clone(),
                        });
                    }
                }
            }
            if let Some(tab) = self.cached_tab(session, 0) {
                return Ok(tab);
            }
        }
    }

    /// Whether the daemon is still starting the session.
    async fn starting(&self, session: &SessionId) -> Result<bool, DaemonError> {
        let session = self
            .sessions()
            .get_session(pb::GetSessionRequest {
                id: Some(pb_session(session)),
            })
            .await
            .map_err(|status| self.error(&status, "The session"))?
            .into_inner();
        Ok(session.status == pb::SessionStatus::Starting as i32)
    }
}

/// The grid a link asks for until its view reports its own size.
const LINK_SIZE: (u16, u16) = (80, 24);

/// Sends one line over a link of its own and waits until the daemon has
/// written it. The first frame shows the attach is up.
async fn type_line(mut link: TerminalLink, route: u64, bytes: Vec<u8>) -> Result<(), DaemonError> {
    let refused = |reason: String| DaemonError::Refused {
        what: "Typing into the terminal".to_owned(),
        reason,
    };
    let ended = || {
        refused("its connection ended before the line was written. Open the tab again".to_owned())
    };
    // A refusal that says what to migrate first comes as a notice just
    // before the error frame, and is what the caller gets (D377).
    fn why(
        notices: &mut tokio::sync::mpsc::Receiver<TerminalNotice>,
        otherwise: DaemonError,
    ) -> DaemonError {
        while let Ok(notice) = notices.try_recv() {
            if let TerminalNotice::Refused(error) = notice {
                return error;
            }
        }
        otherwise
    }
    // Typing waits for the screen (the link refuses it before then).
    loop {
        match link.frames.recv().await {
            Some(ServerFrame::Error { message, .. }) => {
                return Err(why(&mut link.notices, refused(message)));
            }
            Some(ServerFrame::Snapshot { .. }) => break,
            Some(_) => {}
            None => return Err(why(&mut link.notices, ended())),
        }
    }
    let frame = ClientFrame::Input {
        id: route,
        seq: 1,
        bytes,
    };
    link.sink
        .send(frame)
        .map_err(|error| refused(error.to_string()))?;
    while let Some(frame) = link.frames.recv().await {
        match frame {
            ServerFrame::InputWritten { seq: 1, .. } => return Ok(()),
            ServerFrame::Error { message, .. } => {
                return Err(why(&mut link.notices, refused(message)));
            }
            _ => {}
        }
    }
    Err(why(&mut link.notices, ended()))
}

/// Opens a new shell tab of the session, in `folder` when it is not empty,
/// and returns its index among the session's tabs.
async fn new_shell_tab(
    shared: &Arc<Shared>,
    id: &pb::SessionId,
    folder: String,
) -> Result<usize, DaemonError> {
    let tab = shared
        .terminal()
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: fresh_key(),
            session: Some(id.clone()),
            repo: None,
            folder,
        })
        .await
        .map_err(|status| shared.error(&status, "The new shell"))?
        .into_inner();
    let tabs = shared.refresh_tabs(id).await?;
    tabs.iter()
        .position(|candidate| candidate.id == tab.id)
        .ok_or_else(|| DaemonError::Refused {
            what: "The new shell".to_owned(),
            reason: "the daemon opened it but doesn't list it. Reload and try again".to_owned(),
        })
}

/// Resolves the 1-based tab index returned by `TerminalService.OpenAgentTab`:
/// uses `reply.index` when the daemon reported it, else locates `reply.tab.id`
/// in `tabs` (ordered with tab 0 first).
fn resolve_agent_tab_index(
    reply: &pb::OpenAgentTabResponse,
    tabs: &[pb::Tab],
    agent: Agent,
) -> Result<usize, DaemonError> {
    if let Ok(index) = usize::try_from(reply.index)
        && index > 0
    {
        return Ok(index);
    }
    let wanted_id = reply.tab.as_ref().and_then(|tab| tab.id.as_ref());
    tabs.iter()
        .position(|candidate| candidate.id.as_ref() == wanted_id && wanted_id.is_some())
        .ok_or_else(|| DaemonError::Refused {
            what: format!("Starting {} in a new tab", agent.name()),
            reason: "the daemon opened the tab but doesn't list it. Reload and try again"
                .to_owned(),
        })
}

/// Opens a link to tab `tab` of `session` whose attaches open at `size`;
/// (0, 0) leaves the tab at its own size.
fn link_at(shared: &Arc<Shared>, session: &SessionId, tab: usize, size: (u16, u16)) -> Reply<TerminalLink> {
    let shared = shared.clone();
    let session = session.clone();
    Box::pin(async move {
        let tab_id = shared.tab(&session, tab).await?.id.unwrap_or_default();
        Ok(crate::link::start(&shared, pb_session(&session), tab_id, tab as u64, size))
    })
}

impl Terminals for LiveDaemon {
    fn open(&self, session: &SessionId, agent: Agent) -> Reply<(usize, Session)> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move {
            let id = pb_session(&session);
            let index = if agent == Agent::Shell {
                new_shell_tab(&shared, &id, String::new()).await?
            } else {
                let current = shared.fetch_session(&id).await?;
                if current.agent != agent {
                    return Err(DaemonError::Refused {
                        what: format!("Starting {} in this session", agent.name()),
                        reason: format!(
                            "the session runs {}, and a second agent comes in a later release of Humata. Start a new session for {} instead",
                            current.agent.name(),
                            agent.name()
                        ),
                    });
                }
                // The agent's terminal is tab 0, and it is there only once
                // the start has launched the agent: until then the session
                // has nothing to attach, so this waits for it (or refuses).
                shared.tab(&session, 0).await?;
                0
            };
            let current = shared.fetch_session(&id).await?;
            Ok((index, current))
        })
    }

    fn open_in(&self, session: &SessionId, agent: Agent, folder: &str) -> Reply<(usize, Session)> {
        let shared = self.shared.clone();
        let session = session.clone();
        let folder = folder.to_owned();
        Box::pin(async move {
            if agent != Agent::Shell {
                return Err(DaemonError::Refused {
                    what: format!("Starting {} in {folder}", agent.name()),
                    reason: "only a shell can start in a folder of its own for now. Open a terminal here and start the agent in it".to_owned(),
                });
            }
            let id = pb_session(&session);
            let index = new_shell_tab(&shared, &id, folder).await?;
            let current = shared.fetch_session(&id).await?;
            Ok((index, current))
        })
    }

    fn open_agent_tab(
        &self,
        session: &SessionId,
        agent: Agent,
        prompt: &str,
    ) -> Reply<(usize, Session)> {
        let shared = self.shared.clone();
        let session = session.clone();
        let prompt = prompt.to_owned();
        Box::pin(async move {
            if agent == Agent::Shell {
                return Err(DaemonError::Refused {
                    what: "Opening an agent tab".to_owned(),
                    reason: "a plain shell is not an agent. Open a Terminal tab instead".to_owned(),
                });
            }
            let id = pb_session(&session);
            let daemon_agent = shared.agents.name(agent);
            let reply = shared
                .terminal()
                .open_agent_tab(pb::OpenAgentTabRequest {
                    idempotency_key: fresh_key(),
                    session: Some(id.clone()),
                    agent: Some(pb::AgentId {
                        value: daemon_agent,
                    }),
                    prompt,
                })
                .await
                .map_err(|status| {
                    shared.error(&status, &format!("Starting {} in a new tab", agent.name()))
                })?
                .into_inner();
            let tabs = shared.refresh_tabs(&id).await?;
            let index = resolve_agent_tab_index(&reply, &tabs, agent)?;
            let current = shared.fetch_session(&id).await?;
            Ok((index, current))
        })
    }

    fn write(&self, session: &SessionId, tab: usize, text: &str) -> Reply<()> {
        let link = self.link(session, tab);
        let route = u64::try_from(tab).unwrap_or_default();
        let bytes = format!("{text}\r").into_bytes();
        Box::pin(async move { type_line(link.await?, route, bytes).await })
    }

    fn link(&self, session: &SessionId, tab: usize) -> Reply<TerminalLink> {
        link_at(&self.shared, session, tab, LINK_SIZE)
    }

    fn link_unsized(&self, session: &SessionId, tab: usize) -> Reply<TerminalLink> {
        // No size: the daemon leaves the tab's grid as it is (D781).
        link_at(&self.shared, session, tab, (0, 0))
    }

    fn paste_image(&self, session: &SessionId, kind: ImageKind, bytes: Vec<u8>) -> Reply<String> {
        let shared = self.shared.clone();
        let session = pb_session(session);
        Box::pin(async move {
            let pasted = shared
                .terminal()
                .paste_image(pb::PasteImageRequest {
                    idempotency_key: fresh_key(),
                    session: Some(session),
                    data: bytes,
                    format: image_format(kind).into(),
                })
                .await
                .map_err(|status| shared.error(&status, "The pasted image"))?
                .into_inner();
            Ok(pasted.path)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab_with(id: &str, kind: pb::tab::Kind) -> pb::Tab {
        pb::Tab {
            id: Some(pb::TabId { value: id.to_owned() }),
            kind: kind as i32,
            ..Default::default()
        }
    }

    #[test]
    fn resolve_agent_tab_index_uses_reported_index_when_nonzero() {
        let reply = pb::OpenAgentTabResponse {
            tab: Some(tab_with("tb_new", pb::tab::Kind::Shell)),
            index: 3,
        };
        assert_eq!(resolve_agent_tab_index(&reply, &[], Agent::Codex), Ok(3));
    }

    #[test]
    fn resolve_agent_tab_index_falls_back_to_tab_position_and_errors_if_missing() {
        let tabs = vec![
            tab_with("tb_0", pb::tab::Kind::Agent),
            tab_with("tb_1", pb::tab::Kind::Shell),
            tab_with("tb_2", pb::tab::Kind::Shell),
        ];
        let found = pb::OpenAgentTabResponse {
            tab: Some(tab_with("tb_2", pb::tab::Kind::Shell)),
            index: 0,
        };
        assert_eq!(resolve_agent_tab_index(&found, &tabs, Agent::Claude), Ok(2));

        let missing = pb::OpenAgentTabResponse {
            tab: Some(tab_with("tb_gone", pb::tab::Kind::Shell)),
            index: 0,
        };
        assert!(matches!(
            resolve_agent_tab_index(&missing, &tabs, Agent::Claude),
            Err(DaemonError::Refused { .. })
        ));
    }
}
