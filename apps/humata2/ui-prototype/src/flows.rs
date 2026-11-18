//! In-memory interaction fixtures. Nothing here starts or stops a process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Delivery {
    Draft,
    Queued,
    Sent,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LineSpan {
    pub(super) start: usize,
    pub(super) end: usize,
}
impl LineSpan {
    fn display(&self) -> String {
        if self.start == self.end {
            self.start.to_string()
        } else {
            format!("{}–{}", self.start, self.end)
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Anchor {
    File,
    Diff {
        old: Option<LineSpan>,
        new: Option<LineSpan>,
        quote: String,
    },
    Markdown {
        bytes: std::ops::Range<usize>,
        lines: LineSpan,
        quote: String,
        source: String,
    },
}
impl Anchor {
    pub(super) fn label(&self) -> String {
        match self {
            Self::File => "Whole file".into(),
            Self::Diff { old, new, .. } => [
                old.as_ref().map(|r| format!("Old L{}", r.display())),
                new.as_ref().map(|r| format!("New L{}", r.display())),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · "),
            Self::Markdown { lines, .. } => format!("Lines {} · Markdown source", lines.display()),
        }
    }
    pub(super) fn quote(&self) -> &str {
        match self {
            Self::File => "",
            Self::Diff { quote, .. } | Self::Markdown { quote, .. } => quote,
        }
    }
    pub(super) fn markdown(
        source: &str,
        bytes: std::ops::Range<usize>,
    ) -> Option<Self> {
        if bytes.is_empty() || source.get(bytes.clone()).is_none() {
            return None;
        }
        let start = source[..bytes.start]
            .bytes()
            .filter(|b| *b == b'\n')
            .count()
            + 1;
        let end = start
            + source[bytes.clone()]
                .trim_end_matches('\n')
                .bytes()
                .filter(|b| *b == b'\n')
                .count();
        let quote = source[bytes.clone()].to_owned();
        Some(Self::Markdown {
            bytes,
            lines: LineSpan { start, end },
            quote,
            source: source.into(),
        })
    }
    pub(super) fn outdated(&self, source: &str) -> bool {
        match self {
            Self::Markdown {
                bytes,
                source: original,
                ..
            } => original.get(bytes.clone()) != source.get(bytes.clone()),
            _ => false,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RowKind {
    Hunk,
    Context,
    Added,
    Deleted,
}
#[derive(Clone, Debug)]
pub(super) struct DiffRow {
    pub(super) kind: RowKind,
    pub(super) text: String,
    pub(super) old: Option<usize>,
    pub(super) new: Option<usize>,
}
pub(super) fn selected_rows(rows: &[DiffRow], a: usize, b: usize) -> Option<Anchor> {
    let rows = rows.get(a.min(b)..=a.max(b))?;
    let selected: Vec<_> = rows
        .iter()
        .filter(|row| row.kind != RowKind::Hunk)
        .collect();
    if selected.is_empty() {
        return None;
    }
    let span = |old: bool| {
        let numbers: Vec<_> = selected
            .iter()
            .filter_map(|row| if old { row.old } else { row.new })
            .collect();
        numbers.first().map(|first| LineSpan {
            start: *first,
            end: *numbers.last().unwrap(),
        })
    };
    let quote = selected
        .iter()
        .map(|row| {
            format!(
                "{} {}",
                match row.kind {
                    RowKind::Added => "+",
                    RowKind::Deleted => "-",
                    _ => " ",
                },
                row.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some(Anchor::Diff {
        old: span(true),
        new: span(false),
        quote,
    })
}
pub(super) fn diff_rows(file: usize) -> Vec<DiffRow> {
    use RowKind::*;
    let (mut old, mut new, changes): (usize, usize, Vec<(RowKind, &str)>) = match file {
        0 => (
            0,
            1,
            vec![
                (Hunk, "@@ -0,0 +1,12 @@"),
                (Added, "CREATE TABLE ledger_entries ("),
                (Added, "    id          UUID PRIMARY KEY,"),
                (Added, "    account_id  UUID NOT NULL,"),
                (Added, "    debit       BIGINT DEFAULT 0,"),
                (Added, "    credit      BIGINT DEFAULT 0,"),
                (Added, "    created_at  TIMESTAMPTZ NOT NULL,"),
                (Added, "    CONSTRAINT balanced_entry"),
                (Added, "      CHECK (debit >= 0 AND credit >= 0)"),
                (Added, ");"),
                (Added, ""),
                (Added, "CREATE INDEX ledger_account_idx"),
                (Added, "    ON ledger_entries (account_id);"),
            ],
        ),
        1 => (
            82,
            82,
            vec![
                (Hunk, "@@ -82,8 +82,10 @@"),
                (Context, "pub async fn post_invoice("),
                (Context, "    tx: &mut Transaction,"),
                (Context, "    invoice: &Invoice,"),
                (Context, ") -> Result<(), PostingError> {"),
                (Deleted, "    save_invoice(tx, invoice).await?;"),
                (Added, "    let entry = Entry::from(invoice);"),
                (Added, "    ledger.post(tx, entry).await?;"),
                (Added, "    save_invoice(tx, invoice).await?;"),
                (Context, "    Ok(())"),
                (Context, "}"),
            ],
        ),
        _ => (
            24,
            24,
            vec![
                (Hunk, "@@ -24,2 +24,5 @@"),
                (Context, "## Rollback"),
                (Deleted, "Drop the new tables immediately."),
                (Added, "Existing invoices remain readable."),
                (Added, "Revert the posting path before dropping"),
                (Added, "the new ledger tables."),
            ],
        ),
    };
    changes
        .into_iter()
        .map(|(kind, text)| {
            let old_line = matches!(kind, Context | Deleted).then_some(old);
            let new_line = matches!(kind, Context | Added).then_some(new);
            if old_line.is_some() {
                old += 1;
            }
            if new_line.is_some() {
                new += 1;
            }
            DiffRow {
                kind,
                text: text.into(),
                old: old_line,
                new: new_line,
            }
        })
        .collect()
}
/// Small LCS diff for editable prototype documents. Keeps both revision line coordinates.
pub(super) fn document_diff(before: &str, after: &str) -> Vec<DiffRow> {
    use RowKind::*;
    let old: Vec<_> = before.lines().collect();
    let new: Vec<_> = after.lines().collect();
    let mut lengths = vec![vec![0usize; new.len() + 1]; old.len() + 1];
    for a in (0..old.len()).rev() {
        for b in (0..new.len()).rev() {
            lengths[a][b] = if old[a] == new[b] {
                lengths[a + 1][b + 1] + 1
            } else {
                lengths[a + 1][b].max(lengths[a][b + 1])
            };
        }
    }
    let (mut a, mut b) = (0, 0);
    let mut rows = vec![];
    while a < old.len() || b < new.len() {
        let kind = if a < old.len() && b < new.len() && old[a] == new[b] {
            Context
        } else if a < old.len() && (b == new.len() || lengths[a + 1][b] >= lengths[a][b + 1]) {
            Deleted
        } else {
            Added
        };
        rows.push(DiffRow {
            kind,
            text: if kind == Added { new[b] } else { old[a] }.into(),
            old: (kind != Added).then_some(a + 1),
            new: (kind != Deleted).then_some(b + 1),
        });
        if kind != Added {
            a += 1;
        }
        if kind != Deleted {
            b += 1;
        }
    }
    rows
}
pub(super) struct DiffComment {
    pub(super) session: usize,
    pub(super) file: usize,
    pub(super) anchor: Anchor,
    pub(super) owner: usize,
    pub(super) body: String,
    pub(super) delivery: Delivery,
}
#[derive(Clone)]
pub(super) struct Terminals {
    pub(super) agent_hidden: bool,
    pub(super) shell_hidden: bool,
    pub(super) resume_available: bool,
    pub(super) deleted: bool,
}
impl Default for Terminals {
    fn default() -> Self {
        Self {
            agent_hidden: false,
            shell_hidden: false,
            resume_available: true,
            deleted: false,
        }
    }
}
pub(super) const FILES: [(&str, &str); 5] = [
    ("acme/ledger", "migrations/004_ledger.sql"),
    ("acme/api", "src/billing/service.rs"),
    ("acme/docs", "specs/billing-v2/SPEC.md"),
    ("acme/docs", "documents/release-notes.md"),
    ("acme/docs", "notes/rollout-research.md"),
];
pub(super) fn feedback(comments: &[DiffComment], session: usize) -> String {
    let mut body = String::from("Please address this review feedback:\n");
    for comment in comments
        .iter()
        .filter(|c| c.session == session && c.delivery == Delivery::Queued)
    {
        let (repo, path) = FILES[comment.file];
        body.push_str(&format!(
            "\n{repo}/{path} · {}\n{}\n{}\n",
            comment.anchor.label(),
            comment.anchor.quote(),
            comment.body
        ));
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges_preserve_old_and_new_coordinates_for_deleted_added_and_mixed_selections() {
        let rows = diff_rows(1);
        assert!(matches!(
            selected_rows(&rows, 5, 5),
            Some(Anchor::Diff {
                old: Some(LineSpan { start: 86, end: 86 }),
                new: None,
                ..
            })
        ));
        let mixed = selected_rows(&rows, 5, 7).unwrap();
        assert!(matches!(
            &mixed,
            Anchor::Diff {
                old: Some(LineSpan { start: 86, end: 86 }),
                new: Some(LineSpan { start: 86, end: 87 }),
                ..
            }
        ));
        assert!(mixed.quote().contains("-     save_invoice"));
        assert!(mixed.quote().contains("+     let entry"));
        assert_eq!(mixed, selected_rows(&rows, 7, 5).unwrap());
        assert!(matches!(
            selected_rows(&diff_rows(0), 2, 4),
            Some(Anchor::Diff {
                old: None,
                new: Some(LineSpan { start: 2, end: 4 }),
                ..
            })
        ));
        assert!(selected_rows(&rows, 0, 0).is_none());
    }
    #[test]
    fn document_ranges_keep_the_selected_occurrence_and_detect_changed_context() {
        let source = "# α\n\nRepeated passage.\n\nRepeated passage.\n";
        let start = source.rfind("Repeated").unwrap();
        let anchor =
            Anchor::markdown(source, start..source.len()).unwrap();
        assert!(matches!(
            &anchor,
            Anchor::Markdown {
                lines: LineSpan { start: 5, end: 5 },
                ..
            }
        ));
        assert!(!anchor.outdated(source));
        assert!(anchor.outdated(&source[..start]));
        let before = "# Plan\nRemove this text\n";
        let after = "# Plan\nNew text\nMore context\n";
        let rows = document_diff(before, after);
        assert!(matches!(
            selected_rows(&rows, 1, 3),
            Some(Anchor::Diff {
                old: Some(LineSpan { start: 2, end: 2 }),
                new: Some(LineSpan { start: 2, end: 3 }),
                ..
            })
        ));
    }
    #[test]
    fn feedback_keeps_location_and_never_includes_another_sessions_comments() {
        let notes = vec![
            DiffComment {
                session: 0,
                file: 1,
                anchor: Anchor::Diff {
                    old: None,
                    new: Some(LineSpan { start: 7, end: 7 }),
                    quote: String::new(),
                },
                owner: 0,
                body: "Check idempotency".into(),
                delivery: Delivery::Queued,
            },
            DiffComment {
                session: 0,
                file: 0,
                anchor: Anchor::Diff {
                    old: None,
                    new: Some(LineSpan { start: 3, end: 3 }),
                    quote: String::new(),
                },
                owner: 0,
                body: "Add a ledger index".into(),
                delivery: Delivery::Queued,
            },
            DiffComment {
                session: 1,
                file: 0,
                anchor: Anchor::File,
                owner: 0,
                body: "Private to another fork".into(),
                delivery: Delivery::Queued,
            },
            DiffComment {
                session: 0,
                file: 2,
                anchor: Anchor::File,
                owner: 0,
                body: "Unsent draft".into(),
                delivery: Delivery::Draft,
            },
        ];
        let body = feedback(&notes, 0);
        assert!(body.contains("acme/api/src/billing/service.rs · New L7"));
        assert!(body.contains("acme/ledger/migrations/004_ledger.sql · New L3"));
        assert!(body.contains("Check idempotency"));
        assert!(!body.contains("Private to another fork"));
        assert!(!body.contains("Unsent draft"));
    }
}
