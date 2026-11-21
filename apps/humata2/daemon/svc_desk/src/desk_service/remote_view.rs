//! What a Desk session on another machine shows the host, and what the host
//! does when that machine restores one from its Bin (MV-17, D316). The
//! host's folders hold nothing of such a session, so its repos and Markdown
//! files are read from the machine when a list or a watch asks, and the
//! machine's answer is not kept: nothing polls. A machine that can't be
//! reached shows none, and the list still answers.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::pb;
use humata2_sessions::documents_under;
use humata2_store::SessionRecord;
use tonic::Status;

use super::{DeskSvc, now_ms};
use crate::ports::blocking;
use crate::ports::assigned;
use humata2_daemon_ports::Caller;
use crate::ports as identity;
use crate::ports::call_failed;

/// A machine's repos, Markdown files and versions state (whether its folder
/// has versions, and the newest version's id) for each Desk session it lists.
type Lists = HashMap<
    String,
    (Vec<pb::RepoRef>, Vec<String>, bool, String, u32, bool, HashMap<String, u32>),
>;

impl DeskSvc {
    /// What `machine` lists for `asked`, as it shows each session.
    pub(super) async fn listed_on(
        &self,
        machine: &str,
        asked: pb::ListDeskSessionsRequest,
    ) -> Result<Vec<pb::DeskSession>, Status> {
        let mut desk = self.desk_on(machine)?;
        let listed = desk
            .list_desk_sessions(assigned(asked, None).map_err(Status::from)?)
            .await
            .map_err(|source| Status::from(call_failed(machine, "list its Desk sessions", source)))?;
        Ok(listed.into_inner().sessions)
    }

    /// `shown` with the repos and documents of each session that works on
    /// another machine read from that machine, which answers `asked` once.
    /// The machine of each is beside it; none for a session here.
    pub(super) async fn fill_remote(
        &self,
        shown: Vec<(Option<String>, pb::DeskSession)>,
        asked: pb::ListDeskSessionsRequest,
    ) -> Vec<pb::DeskSession> {
        let mut lists: HashMap<String, Lists> = HashMap::new();
        let mut filled = Vec::with_capacity(shown.len());
        for (machine, mut desk) in shown {
            if let Some(machine) = machine {
                // MV-17: the host's disk holds nothing of a session that works
                // elsewhere; only what its machine answers is shown.
                desk.repos.clear();
                desk.documents.clear();
                desk.has_versions = false;
                desk.doc_head.clear();
                if !lists.contains_key(&machine) {
                    let there = self.listed_on(&machine, asked.clone()).await.unwrap_or_default();
                    let mine = there
                        .into_iter()
                        .filter_map(|theirs| {
                            Some((theirs.session?.value, (theirs.repos, theirs.documents, theirs.has_versions, theirs.doc_head, theirs.version_count, theirs.holds_files, theirs.document_lines)))
                        })
                        .collect();
                    lists.insert(machine.clone(), mine);
                }
                let found = desk
                    .session
                    .as_ref()
                    .and_then(|id| lists.get(&machine).and_then(|mine| mine.get(&id.value)));
                if let Some((repos, documents, has_versions, doc_head, version_count, holds_files, lines)) = found {
                    desk.repos = repos.clone();
                    desk.documents = documents.clone();
                    desk.has_versions = *has_versions;
                    desk.doc_head = doc_head.clone();
                    desk.version_count = *version_count;
                    desk.holds_files = *holds_files;
                    desk.document_lines = lines.clone();
                }
            }
            filled.push(desk);
        }
        filled
    }

    /// The Markdown files in `session`'s folder, wherever it works. A
    /// machine that can't be reached is an error, not an empty folder.
    pub(super) async fn documents_of(&self, session: &SessionRecord, channel: Option<&str>) -> Result<Vec<String>, Status> {
        let Some(machine) = self.elsewhere(session) else {
            return Ok(documents_under(&session.folder));
        };
        let asked = pb::ListDeskSessionsRequest {
            channel: channel.map(|value| pb::ChannelId { value: value.to_owned() }),
            target: None,
        };
        let listed = self.listed_on(&machine, asked).await?;
        Ok(listed
            .into_iter()
            .find(|shown| shown.session.as_ref().is_some_and(|id| id.value == session.id))
            .map(|shown| shown.documents)
            .unwrap_or_default())
    }

    /// The host's Bin view restored `item`, an item of another machine's
    /// Bin, on that machine. When it is a channel Desk session's folder, the
    /// host's row opens again: no longer closed, and its folder as the
    /// machine says. Does nothing for any other item.
    pub async fn restored_there(&self, caller: &Caller, item: &str) -> Result<(), Status> {
        let Some(found) = self.closed_into(item).await? else {
            return Ok(());
        };
        let id = found.session_id;
        let _held = self.locks.hold(&id).await;
        let (mut session, mut row) = self.load_channel_desk(&id).await?;
        if row.trash_item.as_deref() != Some(item) {
            // Another restore of this item got here first.
            return Ok(());
        }
        row.closed_unix_ms = None;
        row.trash_item = None;
        row.last_used_unix_ms = now_ms();
        if let Some(machine) = self.elsewhere(&session) {
            let asked = pb::ListDeskSessionsRequest {
                channel: Some(pb::ChannelId { value: row.channel.clone() }),
                target: None,
            };
            let folder = self
                .listed_on(&machine, asked)
                .await
                .ok()
                .and_then(|listed| {
                    listed
                        .into_iter()
                        .find(|shown| shown.session.as_ref().is_some_and(|shown| shown.value == id))
                })
                .and_then(|shown| shown.folder)
                .map(|folder| PathBuf::from(folder.path));
            if let Some(folder) = folder.filter(|folder| *folder != session.folder) {
                session.folder = folder;
                let (store, saving) = (self.starter.store.clone(), session.clone());
                blocking(move || store.put_session(&saving)).await?.map_err(|error| {
                    crate::ports::store_error(error, &self.machine, "save where the restored Desk session works")
                })?;
            }
        }
        self.save_channel_desk_row(&row).await?;
        self.tell
            .session(&id, pb::Change::Updated, Some(identity::actor(caller)))
            .await;
        Ok(())
    }
}
