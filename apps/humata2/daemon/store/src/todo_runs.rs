//! Agent runs and cost ledger recorded against a to-do item.

use rusqlite::params;

use crate::Store;
use crate::StoreError;
use crate::store::check_id;

/// One recorded agent run on a to-do item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TodoRunRecord {
    pub id: String,
    pub item_key: String,
    pub tool: String,
    pub model: String,
    pub outcome: String,
    pub cost_micros: u64,
    pub started_unix_ms: i64,
    pub finished_unix_ms: i64,
    /// JSON-encoded array of `{ "label": "...", "target": "..." }` links.
    pub links_json: String,
    pub session: String,
}

impl Store {
    /// Records or updates a run entry for a to-do item.
    pub fn record_todo_run(&self, run: &TodoRunRecord) -> Result<(), StoreError> {
        check_id(&run.id)?;
        check_id(&run.item_key)?;
        let cost = i64::try_from(run.cost_micros).unwrap_or(i64::MAX);
        let links = if run.links_json.is_empty() {
            "[]"
        } else {
            run.links_json.as_str()
        };
        self.inner()
            .write("todo.run.record", "record a to-do agent run", |transaction| {
                transaction
                    .execute(
                        "INSERT INTO todo_runs (
                             id, item_key, tool, model, outcome, cost_micros,
                             started_unix_ms, finished_unix_ms, links_json, session
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                         ON CONFLICT (id) DO UPDATE SET
                             item_key = excluded.item_key,
                             tool = excluded.tool,
                             model = excluded.model,
                             outcome = excluded.outcome,
                             cost_micros = excluded.cost_micros,
                             started_unix_ms = excluded.started_unix_ms,
                             finished_unix_ms = excluded.finished_unix_ms,
                             links_json = excluded.links_json,
                             session = excluded.session",
                        params![
                            run.id,
                            run.item_key,
                            run.tool,
                            run.model,
                            run.outcome,
                            cost,
                            run.started_unix_ms,
                            run.finished_unix_ms,
                            links,
                            run.session,
                        ],
                    )
                    .map(drop)
            })
    }

    /// Lists all recorded runs for `item_key`, oldest first.
    pub fn list_todo_runs(&self, item_key: &str) -> Result<Vec<TodoRunRecord>, StoreError> {
        check_id(item_key)?;
        self.inner()
            .read("list to-do agent runs", |connection| {
                let mut stmt = connection.prepare(
                    "SELECT id, item_key, tool, model, outcome, cost_micros,
                            started_unix_ms, finished_unix_ms, links_json, session
                     FROM todo_runs
                     WHERE item_key = ?1
                     ORDER BY started_unix_ms ASC, id ASC",
                )?;
                let rows = stmt.query_map([item_key], |row| {
                    let cost_i64: i64 = row.get(5)?;
                    Ok(TodoRunRecord {
                        id: row.get(0)?,
                        item_key: row.get(1)?,
                        tool: row.get(2)?,
                        model: row.get(3)?,
                        outcome: row.get(4)?,
                        cost_micros: u64::try_from(cost_i64.max(0)).unwrap_or(0),
                        started_unix_ms: row.get(6)?,
                        finished_unix_ms: row.get(7)?,
                        links_json: row.get(8)?,
                        session: row.get(9)?,
                    })
                })?;
                rows.collect()
            })
    }
}
