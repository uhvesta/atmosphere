//! The fake's Settings › General › To-do (TODO2WIRE): an in-memory section.
//! A test makes the next write fail with
//! `fail_next(Operation::SetTodoAutomation, ..)`.

use super::FakeDaemon;
use crate::daemon::{Operation, Reply, TodoAutomation};

pub(super) fn todo_automation(daemon: &FakeDaemon) -> Reply<TodoAutomation> {
    daemon.reply(None, "todo_automation".to_owned(), |inner| Ok(inner.todo_automation))
}

pub(super) fn set_todo_automation(daemon: &FakeDaemon, section: TodoAutomation) -> Reply<TodoAutomation> {
    daemon.reply(
        Some(Operation::SetTodoAutomation),
        format!("set_todo_automation {section:?}"),
        move |inner| {
            inner.todo_automation = section;
            Ok(inner.todo_automation)
        },
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tca::TestClock;

    use super::*;
    use crate::daemon::{Daemon, TodoMode};
    use crate::fixture;

    fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
        match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime.block_on(future),
            Err(error) => panic!("test runtime: {error}"),
        }
    }

    fn daemon() -> FakeDaemon {
        FakeDaemon::new(fixture::design(), Arc::new(TestClock::new()))
    }

    #[test]
    fn a_saved_section_is_what_is_read_back() {
        let daemon = daemon();
        let section = TodoAutomation { on_pr_merged: TodoMode::Auto, ..TodoAutomation::default() };
        assert_eq!(block_on(daemon.set_todo_automation(section)), Ok(section));
        assert_eq!(block_on(daemon.todo_automation()), Ok(section));
    }

    #[test]
    fn a_refused_write_leaves_the_section_as_it_was() {
        let daemon = daemon();
        daemon.fail_next(Operation::SetTodoAutomation, "no");
        let section = TodoAutomation { auto_wake_on_ci_fail: true, ..TodoAutomation::default() };
        assert!(block_on(daemon.set_todo_automation(section)).is_err());
        assert_eq!(block_on(daemon.todo_automation()), Ok(TodoAutomation::default()));
    }
}
