//! Setting up a channel on this machine: its record, folders and main
//! clones (CH-5, FC-1, FC-2).

use std::time::SystemTime;

use humata2_store::fault::point;
use humata2_store::{ChannelRecord, ChannelRepo, MainClone, Store, StoreError};

use crate::error::{GitError, SetupError};
use crate::git::{clone_main, remote_url};
use crate::layout::Layout;
use crate::names::channel_folder;

/// A channel to create.
#[derive(Clone, Debug)]
pub struct NewChannel {
    pub name: String,
    /// `(owner, name)`, in the order the user gave.
    pub repos: Vec<(String, String)>,
    pub machines: Vec<String>,
    pub branch_prefix: String,
    pub default_agent: String,
}

pub(crate) fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

pub(crate) fn store_error(doing: &'static str) -> impl Fn(StoreError) -> SetupError {
    move |source| SetupError::Store { doing, source }
}

/// Saves the channel with every repository still to be cloned, and makes
/// its folders. Clone the mains with [`clone_mains`].
pub fn create_channel(
    store: &Store,
    layout: &Layout,
    new: NewChannel,
) -> Result<ChannelRecord, SetupError> {
    let folder = channel_folder(&new.name)?;
    let existing = store.channels().map_err(store_error("list the channels"))?;
    if let Some(other) = existing.iter().find(|c| c.folder == folder) {
        return Err(SetupError::FolderTaken {
            folder,
            other: other.name.clone(),
        });
    }
    let record = ChannelRecord {
        id: format!("ch_{}", uuid::Uuid::new_v4().simple()),
        name: new.name,
        folder,
        branch_prefix: new.branch_prefix,
        default_agent: new.default_agent,
        repos: new
            .repos
            .into_iter()
            .map(|(owner, name)| ChannelRepo {
                owner,
                name,
                main: MainClone::Cloning,
            })
            .collect(),
        machines: new.machines,
        revision: 1,
        created_unix_ms: unix_ms(),
    };
    store
        .put_channel(&record)
        .map_err(store_error("save the channel"))?;
    point("sessions.channel.recorded");
    layout.create_channel(&record.folder)?;
    Ok(record)
}

/// Clones the main checkout of each repository of `channel_id` that is
/// still cloning, and records Ready or Failed for each. Blocking. A
/// repository whose main clone is already on disk is just marked Ready,
/// so running it again after a crash finishes the work. `on_step` hears
/// each repository (`owner/name`) and how its clone went.
pub fn clone_mains(
    store: &Store,
    layout: &Layout,
    remote_template: &str,
    channel_id: &str,
    mut on_step: impl FnMut(&str, Result<(), &GitError>),
) -> Result<(), SetupError> {
    let channel = store
        .channel(channel_id)
        .map_err(store_error("read the channel"))?
        .ok_or_else(|| SetupError::NoChannel {
            id: channel_id.to_owned(),
        })?;
    for repo in channel
        .repos
        .iter()
        .filter(|r| r.main == MainClone::Cloning)
    {
        let dest = layout.main_clone(&channel.folder, &repo.name);
        let shown = format!("{}/{}", repo.owner, repo.name);
        let result = if dest.join(".git").exists() {
            Ok(())
        } else {
            remote_url(remote_template, &repo.owner, &repo.name)
                .and_then(|url| clone_main(&url, &dest))
        };
        point("sessions.channel.cloned");
        let main = match &result {
            Ok(()) => MainClone::Ready,
            Err(error) => MainClone::Failed {
                reason: error.to_string(),
            },
        };
        store
            .set_main_clone(&channel.id, &repo.owner, &repo.name, &main)
            .map_err(store_error("record a main clone"))?;
        on_step(&shown, result.as_ref().map(|_| ()).map_err(|e| e));
    }
    Ok(())
}
