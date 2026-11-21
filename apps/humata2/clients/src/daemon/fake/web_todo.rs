//! Fake daemon backing for the To-do web UI switch (WEBTODO, D905).

use super::FakeDaemon;
use crate::daemon::{Operation, Reply, WebTodoSnapshot, WebTodoStatus};

pub(super) fn web_todo_settings(daemon: &FakeDaemon) -> Reply<WebTodoSnapshot> {
    daemon.reply(None, "web_todo_settings".to_owned(), |inner| {
        Ok(inner.web_todo.clone())
    })
}

pub(super) fn set_web_todo_enabled(daemon: &FakeDaemon, enabled: bool) -> Reply<WebTodoSnapshot> {
    daemon.reply(
        Some(Operation::SetWebTodoEnabled),
        format!("set_web_todo_enabled {enabled}"),
        move |inner| {
            inner.web_todo.enabled = enabled;
            inner.web_todo.status = if enabled {
                WebTodoStatus::Listening {
                    url: "http://127.0.0.1:17420/".to_owned(),
                }
            } else {
                WebTodoStatus::Off
            };
            Ok(inner.web_todo.clone())
        },
    )
}
