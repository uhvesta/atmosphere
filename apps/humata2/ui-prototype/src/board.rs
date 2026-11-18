//! Prototype board data. UI actions and drag/drop use the same transitions.
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Status {
    Backlog,
    Todo,
    InProgress,
    InReview,
    Done,
    Blocked,
    WontDo,
}
impl Status {
    pub(super) const COLUMNS: [Self; 7] = [
        Self::Backlog,
        Self::Todo,
        Self::InProgress,
        Self::Blocked,
        Self::InReview,
        Self::Done,
        Self::WontDo,
    ];
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Backlog => "Backlog",
            Self::Todo => "To do",
            Self::InProgress => "In progress",
            Self::InReview => "In review",
            Self::Done => "Done",
            Self::Blocked => "Blocked",
            Self::WontDo => "Won’t do",
        }
    }
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Backlog => "backlog",
            Self::Todo => "todo",
            Self::InProgress => "in-progress",
            Self::InReview => "in-review",
            Self::Done => "done",
            Self::Blocked => "blocked",
            Self::WontDo => "wont-do",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Milestone,
    Task,
    Step,
}
impl Kind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Milestone => "Milestone",
            Self::Task => "Task",
            Self::Step => "Step",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Global,
    Channel(usize),
}

pub(super) struct Comment {
    pub(super) body: String,
    pub(super) author: &'static str,
}
pub(super) struct HistoryEntry {
    pub(super) author: &'static str,
    pub(super) change: Change,
}
pub(super) enum Change {
    Created,
    Moved {
        from: Status,
        to: Status,
    },
    Edited {
        before_title: String,
        after_title: String,
        before_details: String,
        after_details: String,
    },
    BlockersSet {
        ids: Vec<String>,
        note: String,
    },
    Commented {
        body: String,
    },
}
impl Change {
    pub(super) fn summary(&self) -> String {
        match self {
            Self::Created => "Created the item".into(),
            Self::Moved { from, to } => format!("{} → {}", from.label(), to.label()),
            Self::Edited {
                before_title,
                after_title,
                before_details,
                after_details,
            } => {
                let mut changes = vec![];
                if before_title != after_title {
                    changes.push(format!("Title: {before_title} → {after_title}"));
                }
                if before_details != after_details {
                    changes.push(format!("Description updated:\n{after_details}"));
                }
                changes.join("\n")
            }
            Self::BlockersSet { ids, note } => format!(
                "Set blockers: {}{}{}",
                ids.join(", "),
                if ids.is_empty() || note.is_empty() {
                    ""
                } else {
                    " · "
                },
                note
            ),
            Self::Commented { body } => format!("Commented: {body}"),
        }
    }
}

pub(super) struct Item {
    pub(super) id: String,
    pub(super) kind: Kind,
    pub(super) scope: Scope,
    pub(super) parent: Option<usize>,
    pub(super) session: Option<usize>,
    pub(super) title: String,
    pub(super) details: String,
    pub(super) status: Status,
    pub(super) blocked_on: Vec<usize>,
    pub(super) blocker_note: String,
    pub(super) comments: Vec<Comment>,
    pub(super) history: Vec<HistoryEntry>,
}
pub(super) struct Board {
    pub(super) items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BoardError {
    EmptyTitle { item: usize },
    InvalidParent { parent: Option<usize> },
    EmptyComment { item: usize },
    MissingBlocker { item: usize },
    SelfDependency { item: usize },
    DependencyCycle { item: usize, prerequisite: usize },
}
impl fmt::Display for BoardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidParent { .. } => "A step must belong to a milestone. Tasks and milestones stand alone.",
            Self::EmptyTitle { .. } => "Add a title before saving the item.",
            Self::EmptyComment { .. } => "Write a comment before adding it.",
            Self::MissingBlocker { .. } => "Choose a prerequisite item or describe an external blocker.",
            Self::SelfDependency { .. } => "An item cannot block itself. Choose another item.",
            Self::DependencyCycle { .. } => "That prerequisite already depends on this item. Choose another to avoid a dependency cycle.",
        })
    }
}
impl std::error::Error for BoardError {}

impl Default for Board {
    fn default() -> Self {
        let seeds = [
            (
                "G-1",
                "Audit invoice edge cases",
                "Check rounding, duplicate webhooks and mixed-currency invoices. Record any cases the existing tests miss.",
                Status::Backlog,
            ),
            (
                "PAY-9",
                "Try range partitioning",
                "Compare monthly range partitions with the current postings table. Keep the experiment on its own branch.",
                Status::Todo,
            ),
            (
                "PAY-3",
                "Design the ledger schema",
                "Define accounts and integer postings for the double-entry ledger. Include a reversible migration and balance-invariant tests.",
                Status::InReview,
            ),
            (
                "PAY-4",
                "Write the rollback migration",
                "Restore the previous posting path before removing the new tables. Existing invoices must remain readable.",
                Status::InProgress,
            ),
            (
                "PAY-5",
                "Add property test for balance",
                "Generate balanced posting batches and prove that their sum is zero. Reject any batch that violates the invariant.",
                Status::Todo,
            ),
        ];
        let mut board = Self {
            items: seeds
                .into_iter()
                .map(|(id, title, details, status)| Item {
                    id: id.into(),
                    kind: Kind::Task,
                    scope: Scope::Global,
                    parent: None,
                    session: None,
                    title: title.into(),
                    details: details.into(),
                    status,
                    blocked_on: vec![],
                    blocker_note: String::new(),
                    comments: vec![],
                    history: vec![HistoryEntry {
                        author: "You",
                        change: Change::Created,
                    }],
                })
                .collect(),
        };
        let ledger = board
            .create(
                Kind::Milestone,
                Scope::Channel(0),
                None,
                "Ship the new ledger".into(),
            )
            .unwrap();
        board.items[ledger].details =
            "Deliver a reversible double-entry ledger with tested balance invariants.".into();
        board.items[ledger].status = Status::InProgress;
        for index in [2, 3, 4] {
            board.items[index].kind = Kind::Step;
            board.items[index].scope = Scope::Channel(0);
            board.items[index].parent = Some(ledger);
            board.items[index].session = Some(0);
        }
        board.items[1].scope = Scope::Channel(0);
        board.items[1].session = Some(1);
        let launch = board
            .create(
                Kind::Milestone,
                Scope::Global,
                None,
                "Prepare the release".into(),
            )
            .unwrap();
        board.items[launch].details = "Coordinate release readiness across projects.".into();
        board
            .create(
                Kind::Step,
                Scope::Global,
                Some(launch),
                "Review the release checklist".into(),
            )
            .unwrap();
        board
            .create(
                Kind::Task,
                Scope::Channel(1),
                None,
                "Fix dashboard empty state".into(),
            )
            .unwrap();
        board
    }
}
impl Board {
    pub(super) fn create(
        &mut self,
        kind: Kind,
        scope: Scope,
        parent: Option<usize>,
        title: String,
    ) -> Result<usize, BoardError> {
        let index = self.items.len();
        if title.trim().is_empty() {
            return Err(BoardError::EmptyTitle { item: index });
        }
        if (kind == Kind::Step
            && !parent.is_some_and(|p| {
                self.items
                    .get(p)
                    .is_some_and(|item| item.kind == Kind::Milestone)
            }))
            || (kind != Kind::Step && parent.is_some())
        {
            return Err(BoardError::InvalidParent { parent });
        }
        let scope = parent.map(|p| self.items[p].scope).unwrap_or(scope);
        self.items.push(Item {
            id: format!(
                "{}-{}",
                match scope {
                    Scope::Global => "G",
                    Scope::Channel(0) => "PAY",
                    Scope::Channel(_) => "WORK",
                },
                index + 10
            ),
            kind,
            scope,
            parent,
            session: None,
            title: title.trim().into(),
            details: String::new(),
            status: Status::Todo,
            blocked_on: vec![],
            blocker_note: String::new(),
            comments: vec![],
            history: vec![HistoryEntry {
                author: "You",
                change: Change::Created,
            }],
        });
        Ok(index)
    }
    pub(super) fn children(&self, parent: usize) -> Vec<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.parent == Some(parent))
            .map(|(i, _)| i)
            .collect()
    }

    pub(super) fn move_item(&mut self, item: usize, to: Status) {
        let from = self.items[item].status;
        if from == to {
            return;
        }
        self.items[item].status = to;
        self.items[item].history.push(HistoryEntry {
            author: "You",
            change: Change::Moved { from, to },
        });
    }
    pub(super) fn edit(
        &mut self,
        item: usize,
        title: String,
        details: String,
    ) -> Result<(), BoardError> {
        let title = title.trim().to_owned();
        if title.is_empty() {
            return Err(BoardError::EmptyTitle { item });
        }
        let entry = &mut self.items[item];
        if entry.title == title && entry.details == details {
            return Ok(());
        }
        let change = Change::Edited {
            before_title: entry.title.clone(),
            after_title: title.clone(),
            before_details: entry.details.clone(),
            after_details: details.clone(),
        };
        entry.title = title;
        entry.details = details;
        entry.history.push(HistoryEntry {
            author: "You",
            change,
        });
        Ok(())
    }
    pub(super) fn comment(&mut self, item: usize, body: String) -> Result<(), BoardError> {
        let body = body.trim().to_owned();
        if body.is_empty() {
            return Err(BoardError::EmptyComment { item });
        }
        self.items[item].comments.push(Comment {
            body: body.clone(),
            author: "You",
        });
        self.items[item].history.push(HistoryEntry {
            author: "You",
            change: Change::Commented { body },
        });
        Ok(())
    }
    fn reaches(&self, from: usize, target: usize, seen: &mut Vec<usize>) -> bool {
        if from == target {
            return true;
        }
        if seen.contains(&from) {
            return false;
        }
        seen.push(from);
        self.items[from]
            .blocked_on
            .iter()
            .any(|next| self.reaches(*next, target, seen))
    }
    pub(super) fn block(
        &mut self,
        item: usize,
        mut on: Vec<usize>,
        note: String,
    ) -> Result<(), BoardError> {
        let note = note.trim().to_owned();
        if on.is_empty() && note.is_empty() {
            return Err(BoardError::MissingBlocker { item });
        }
        for prerequisite in &on {
            if *prerequisite == item {
                return Err(BoardError::SelfDependency { item });
            }
            if self.reaches(*prerequisite, item, &mut vec![]) {
                return Err(BoardError::DependencyCycle {
                    item,
                    prerequisite: *prerequisite,
                });
            }
        }
        on.sort_unstable();
        on.dedup();
        let ids = on.iter().map(|i| self.items[*i].id.clone()).collect();
        self.items[item].blocked_on = on;
        self.items[item].blocker_note = note.clone();
        self.items[item].history.push(HistoryEntry {
            author: "You",
            change: Change::BlockersSet { ids, note },
        });
        self.move_item(item, Status::Blocked);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn steps_inherit_milestone_scope_and_standalone_tasks_cannot_have_parents() {
        let mut board = Board::default();
        let milestone = board
            .create(
                Kind::Milestone,
                Scope::Channel(1),
                None,
                "Ship the dashboard".into(),
            )
            .unwrap();
        let step = board
            .create(
                Kind::Step,
                Scope::Global,
                Some(milestone),
                "Add empty states".into(),
            )
            .unwrap();
        assert_eq!(board.items[step].scope, Scope::Channel(1));
        assert_eq!(board.items[step].parent, Some(milestone));
        assert!(
            board
                .create(
                    Kind::Task,
                    Scope::Global,
                    Some(milestone),
                    "Standalone".into()
                )
                .is_err()
        );
        assert!(
            board
                .create(Kind::Step, Scope::Global, None, "Orphan".into())
                .is_err()
        );
    }
    #[test]
    fn blocked_items_can_reference_prerequisites_or_external_reasons() {
        let mut board = Board::default();
        board.block(0, vec![1], String::new()).unwrap();
        assert_eq!(board.items[0].blocked_on, vec![1]);
        assert_eq!(board.items[0].status, Status::Blocked);
        board
            .block(2, vec![], "Waiting for the database team".into())
            .unwrap();
        assert_eq!(board.items[2].blocker_note, "Waiting for the database team");
    }
    #[test]
    fn cycles_and_missing_blockers_leave_the_item_unchanged() {
        let mut board = Board::default();
        assert!(matches!(
            board.block(0, vec![], String::new()),
            Err(BoardError::MissingBlocker { item: 0 })
        ));
        assert!(matches!(
            board.block(0, vec![0], String::new()),
            Err(BoardError::SelfDependency { item: 0 })
        ));
        board.block(0, vec![1], String::new()).unwrap();
        assert!(matches!(
            board.block(1, vec![0], String::new()),
            Err(BoardError::DependencyCycle {
                item: 1,
                prerequisite: 0
            })
        ));
        assert_eq!(board.items[1].status, Status::Todo);
        assert!(board.items[1].blocked_on.is_empty());
    }
    #[test]
    fn edits_comments_and_moves_keep_an_item_history() {
        let mut board = Board::default();
        board
            .edit(0, "Audit refunds".into(), "Check duplicate delivery".into())
            .unwrap();
        board
            .comment(0, "Found a duplicate webhook".into())
            .unwrap();
        board.move_item(0, Status::WontDo);
        assert_eq!(board.items[0].title, "Audit refunds");
        assert_eq!(board.items[0].comments[0].body, "Found a duplicate webhook");
        assert!(matches!(
            board.items[0].history.last().unwrap().change,
            Change::Moved {
                to: Status::WontDo,
                ..
            }
        ));
        assert_eq!(board.items[0].history.len(), 4);
    }
}
