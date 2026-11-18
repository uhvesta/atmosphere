//! Channels (CH-1, CH-2, CH-5) and the state of a session's start (FC-1,
//! FC-3, SE-4, PER-11, PER-27), so a restart finds both where they were.

use rusqlite::{Connection, Transaction, params};

use crate::store::{bytes_path, check_id, from_sql, insert_session, path_bytes, to_sql};
use crate::{
    ChannelRecord, ChannelRepo, MainClone, SessionDetail, SessionRecord, SessionRepo, StartStep,
    Store, StoreError,
};

/// Saves `detail` and its clones in `transaction`, in place of what the
/// session had: the statements of [`Store::put_session_detail`], shared with
/// the move of a Desk session so the two can't drift.
pub(crate) fn write_session_detail(
    transaction: &Transaction<'_>,
    detail: &SessionDetail,
    failed: &impl Fn(rusqlite::Error) -> StoreError,
) -> Result<(), StoreError> {
    transaction
        .execute(
            "INSERT INTO session_details
                 (session, model, first_message, start_step, operation, revision)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (session) DO UPDATE SET
                 model = excluded.model, first_message = excluded.first_message,
                 start_step = excluded.start_step, operation = excluded.operation,
                 revision = excluded.revision",
            params![
                detail.session,
                detail.model,
                detail.first_message,
                step_name(detail.start),
                detail.operation,
                to_sql(detail.revision)
            ],
        )
        .map_err(failed)?;
    transaction
        .execute("DELETE FROM session_repos WHERE session = ?1", [&detail.session])
        .map_err(failed)?;
    for (n, repo) in detail.repos.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO session_repos (session, position, owner, name, branch, path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    detail.session,
                    position(n),
                    repo.owner,
                    repo.name,
                    repo.branch,
                    path_bytes(&repo.path)
                ],
            )
            .map_err(failed)?;
    }
    Ok(())
}

fn position(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

fn main_columns(main: &MainClone) -> (&'static str, Option<&str>) {
    match main {
        MainClone::Cloning => ("cloning", None),
        MainClone::Ready => ("ready", None),
        MainClone::Failed { reason } => ("failed", Some(reason)),
    }
}

fn main_clone(state: &str, error: Option<String>) -> MainClone {
    match state {
        "ready" => MainClone::Ready,
        "failed" => MainClone::Failed {
            reason: error.unwrap_or_default(),
        },
        _ => MainClone::Cloning,
    }
}

fn step_name(step: StartStep) -> &'static str {
    match step {
        StartStep::Recorded => "recorded",
        StartStep::Cloned => "cloned",
        StartStep::Launching => "launching",
        StartStep::Done => "done",
    }
}

fn step(name: &str) -> StartStep {
    match name {
        "cloned" => StartStep::Cloned,
        "launching" => StartStep::Launching,
        "done" => StartStep::Done,
        _ => StartStep::Recorded,
    }
}

fn read_channels(
    connection: &Connection,
    only: Option<&str>,
) -> rusqlite::Result<Vec<ChannelRecord>> {
    let mut statement = connection.prepare(
        "SELECT id, name, folder, branch_prefix, default_agent, revision, created_unix_ms,
                branch_prefix_set
         FROM channels WHERE (?1 IS NULL OR id = ?1) ORDER BY created_unix_ms, id",
    )?;
    let mut channels = statement
        .query_map([only], |row| {
            Ok(ChannelRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                folder: row.get(2)?,
                branch_prefix: if row.get::<_, i64>(7)? != 0 { Some(row.get(3)?) } else { None },
                default_agent: row.get(4)?,
                repos: Vec::new(),
                machines: Vec::new(),
                revision: from_sql(row.get(5)?),
                created_unix_ms: row.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for channel in &mut channels {
        channel.repos = connection
            .prepare(
                "SELECT owner, name, main_state, main_error FROM channel_repos
                 WHERE channel = ?1 ORDER BY position",
            )?
            .query_map([&channel.id], |row| {
                Ok(ChannelRepo {
                    owner: row.get(0)?,
                    name: row.get(1)?,
                    main: main_clone(&row.get::<_, String>(2)?, row.get(3)?),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        channel.machines = connection
            .prepare("SELECT machine FROM channel_machines WHERE channel = ?1 ORDER BY position")?
            .query_map([&channel.id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
    }
    Ok(channels)
}

fn read_details(
    connection: &Connection,
    only: Option<&str>,
) -> rusqlite::Result<Vec<SessionDetail>> {
    let mut statement = connection.prepare(
        "SELECT d.session, d.model, d.first_message, d.start_step, d.operation, d.revision
         FROM session_details d JOIN sessions s ON s.id = d.session
         WHERE (?1 IS NULL OR d.session = ?1) ORDER BY s.created_unix_ms, d.session",
    )?;
    let mut details = statement
        .query_map([only], |row| {
            Ok(SessionDetail {
                session: row.get(0)?,
                repos: Vec::new(),
                model: row.get(1)?,
                first_message: row.get(2)?,
                start: step(&row.get::<_, String>(3)?),
                operation: row.get(4)?,
                revision: from_sql(row.get(5)?),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for detail in &mut details {
        detail.repos = connection
            .prepare(
                "SELECT owner, name, branch, path FROM session_repos
                 WHERE session = ?1 ORDER BY position",
            )?
            .query_map([&detail.session], |row| {
                Ok(SessionRepo {
                    owner: row.get(0)?,
                    name: row.get(1)?,
                    branch: row.get(2)?,
                    path: bytes_path(row.get(3)?),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
    }
    Ok(details)
}

impl Store {
    /// Saves a channel, replacing one with its id along with its
    /// repositories and machines, in the order given.
    pub fn put_channel(&self, channel: &ChannelRecord) -> Result<(), StoreError> {
        check_id(&channel.id)?;
        let doing = "save a channel";
        self.inner().write("channel.put", doing, |transaction| {
            transaction.execute(
                "INSERT INTO channels
                     (id, name, folder, branch_prefix, default_agent, revision, created_unix_ms,
                      branch_prefix_set)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT (id) DO UPDATE SET
                     name = excluded.name, folder = excluded.folder,
                     branch_prefix = excluded.branch_prefix,
                     branch_prefix_set = excluded.branch_prefix_set,
                     default_agent = excluded.default_agent,
                     revision = excluded.revision,
                     created_unix_ms = excluded.created_unix_ms",
                params![
                    channel.id,
                    channel.name,
                    channel.folder,
                    channel.branch_prefix.as_deref().unwrap_or(""),
                    channel.default_agent,
                    to_sql(channel.revision),
                    channel.created_unix_ms,
                    i64::from(channel.branch_prefix.is_some())
                ],
            )?;
            transaction.execute("DELETE FROM channel_repos WHERE channel = ?1", [&channel.id])?;
            transaction.execute("DELETE FROM channel_machines WHERE channel = ?1", [&channel.id])?;
            for (n, repo) in channel.repos.iter().enumerate() {
                let (state, error) = main_columns(&repo.main);
                transaction.execute(
                    "INSERT INTO channel_repos (channel, position, owner, name, main_state, main_error)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![channel.id, position(n), repo.owner, repo.name, state, error],
                )?;
            }
            for (n, machine) in channel.machines.iter().enumerate() {
                transaction.execute(
                    "INSERT INTO channel_machines (channel, machine, position) VALUES (?1, ?2, ?3)",
                    params![channel.id, machine, position(n)],
                )?;
            }
            Ok(())
        })
    }

    /// The channel `id`, if one was saved.
    pub fn channel(&self, id: &str) -> Result<Option<ChannelRecord>, StoreError> {
        check_id(id)?;
        let channels = self.inner().read("read a channel", |connection| {
            read_channels(connection, Some(id))
        })?;
        Ok(channels.into_iter().next())
    }

    /// Every channel, oldest first, then by id.
    pub fn channels(&self) -> Result<Vec<ChannelRecord>, StoreError> {
        self.inner().read("list channels", |connection| {
            read_channels(connection, None)
        })
    }

    /// Records how cloning one repository's main checkout stands. Fails
    /// with `NoChannel` or `NoChannelRepo` when there is nothing to update.
    pub fn set_main_clone(
        &self,
        channel: &str,
        owner: &str,
        name: &str,
        main: &MainClone,
    ) -> Result<(), StoreError> {
        check_id(channel)?;
        let doing = "record a main clone's state";
        let (state, error) = main_columns(main);
        self.inner()
            .write_with("channel.main_clone", doing, |transaction| {
                let failed = self.inner().failed(doing);
                let changed = transaction
                    .execute(
                        "UPDATE channel_repos SET main_state = ?4, main_error = ?5
                     WHERE channel = ?1 AND owner = ?2 AND name = ?3",
                        params![channel, owner, name, state, error],
                    )
                    .map_err(&failed)?;
                if changed > 0 {
                    return Ok(());
                }
                let known: i64 = transaction
                    .query_row(
                        "SELECT COUNT(*) FROM channels WHERE id = ?1",
                        [channel],
                        |row| row.get(0),
                    )
                    .map_err(&failed)?;
                Err(if known == 0 {
                    StoreError::NoChannel {
                        id: channel.to_owned(),
                    }
                } else {
                    StoreError::NoChannelRepo {
                        channel: channel.to_owned(),
                        owner: owner.to_owned(),
                        name: name.to_owned(),
                    }
                })
            })
    }

    /// Changes a channel's name and nothing else, and bumps its revision,
    /// in one transaction. With `expected_revision` set, a channel at any
    /// other revision is refused with `StaleChannel`; one that isn't saved
    /// with `NoChannel`. Answers the channel as saved.
    pub fn rename_channel(
        &self,
        id: &str,
        name: &str,
        expected_revision: Option<u64>,
    ) -> Result<ChannelRecord, StoreError> {
        check_id(id)?;
        let doing = "rename a channel";
        self.inner()
            .write_with("channel.rename", doing, |transaction| {
                let failed = self.inner().failed(doing);
                let found = read_channels(transaction, Some(id)).map_err(&failed)?;
                let Some(mut channel) = found.into_iter().next() else {
                    return Err(StoreError::NoChannel { id: id.to_owned() });
                };
                if let Some(expected) = expected_revision
                    && expected != channel.revision
                {
                    return Err(StoreError::StaleChannel {
                        id: id.to_owned(),
                        expected,
                        actual: channel.revision,
                    });
                }
                channel.revision += 1;
                channel.name = name.to_owned();
                transaction
                    .execute(
                        "UPDATE channels SET name = ?2, revision = ?3 WHERE id = ?1",
                        params![id, name, to_sql(channel.revision)],
                    )
                    .map_err(&failed)?;
                Ok(channel)
            })
    }

    /// Removes a channel with its repositories and machines. Its sessions
    /// are kept; removing a channel that isn't saved does nothing.
    pub fn remove_channel(&self, id: &str) -> Result<(), StoreError> {
        check_id(id)?;
        self.inner()
            .write("channel.remove", "remove a channel", |transaction| {
                transaction
                    .execute("DELETE FROM channels WHERE id = ?1", [id])
                    .map(drop)
            })
    }

    /// Saves where a session's start stands, replacing its clones too.
    /// The session must be saved first.
    pub fn put_session_detail(&self, detail: &SessionDetail) -> Result<(), StoreError> {
        let doing = "save a session's start";
        self.inner()
            .write_for_session("session_detail.put", doing, Some(&detail.session), |transaction| {
                write_session_detail(transaction, detail, &self.inner().failed(doing))
            })
    }

    /// Saves a new session and where its start stands in one write, so a
    /// crash leaves both or neither.
    pub fn put_session_with_detail(
        &self,
        session: &SessionRecord,
        detail: &SessionDetail,
    ) -> Result<(), StoreError> {
        check_id(&session.id)?;
        check_id(&detail.session)?;
        let doing = "save a session and its start";
        let failed = self.inner().failed(doing);
        self.inner().write_with("session.put_with_detail", doing, |transaction| {
            insert_session(transaction, session).map_err(&failed)?;
            write_session_detail(transaction, detail, &failed)
        })
    }

    /// The start state of `session`, if one was saved.
    pub fn session_detail(&self, session: &str) -> Result<Option<SessionDetail>, StoreError> {
        check_id(session)?;
        let details = self.inner().read("read a session's start", |connection| {
            read_details(connection, Some(session))
        })?;
        Ok(details.into_iter().next())
    }

    /// Every saved session start, oldest session first.
    pub fn session_details(&self) -> Result<Vec<SessionDetail>, StoreError> {
        self.inner().read("list session starts", |connection| {
            read_details(connection, None)
        })
    }
}
