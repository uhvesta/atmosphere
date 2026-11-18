//! Markdown documents share source, preview, revision review and anchored feedback.
use super::*;
use gpui_kit::component::input::{Editor, EditorState};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Preview,
    Edit,
    Changes,
}
pub(super) struct MarkdownPane {
    pub(super) source: Entity<EditorState>,
    baseline: String,
    preview: crate::rich::Document,
    before: crate::rich::Document,
    mode: Mode,
    rendered_changes: bool,
    pub(super) selection: Option<flows::Anchor>,
    recipient: usize,
    choosing_recipient: bool,
    _subscription: Subscription,
}
fn fixture(file: usize) -> &'static str {
    match file {
        3 => {
            "# Release notes\n\n## What changed\n\nBilling now uses a **double-entry ledger**. Existing invoices stay readable.\n\n## Rollout\n\n1. Deploy the schema.\n2. Backfill existing invoices.\n3. Enable ledger writes.\n\n```json\n{\n  \"ledger_enabled\": false,\n  \"backfill_batch_size\": 1000\n}\n```\n\n## Support notes\n\nAsk the billing team before retrying a failed migration.\n"
        }
        4 => {
            "# Rollout research\n\n## Question\n\nCan we roll back without interrupting invoice creation?\n\n## Findings\n\nExisting invoices remain readable.\n\n```sql\nSELECT account_id, SUM(debit - credit)\nFROM ledger_entries\nGROUP BY account_id;\n```\n\n## Open questions\n\nExisting invoices remain readable.\n\nConfirm whether this also holds for refunds.\n\n```bash\ncargo test -p ledger --test rollback\n```\n"
        }
        _ => crate::rich::SAMPLE,
    }
}
impl Prototype {
    pub(super) fn ensure_document(
        &mut self,
        file: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = (self.active, file);
        if self.documents.contains_key(&key) {
            return;
        }
        let baseline = fixture(file).to_owned();
        let source = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("markdown")
                .default_value(&baseline)
        });
        let subscription = cx.subscribe(&source, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                if let Some(document) = this.documents.get_mut(&key) {
                    document.selection = None;
                }
                cx.notify();
            }
        });
        self.documents.insert(
            key,
            MarkdownPane {
                source,
                preview: crate::rich::Document::new(&baseline),
                before: crate::rich::Document::new(&baseline),
                baseline,
                mode: Mode::Preview,
                rendered_changes: false,
                selection: None,
                recipient: self.active,
                choosing_recipient: false,
                _subscription: subscription,
            },
        );
    }
    pub(super) fn open_document(
        &mut self,
        file: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ensure_document(file, window, cx);
        let already_open = self.review && self.review_tab == "Document";
        self.markdown_file = file;
        self.review = true;
        self.review_tab = "Document";
        if !already_open {
            self.review_full = false;
        }
        self.comment_open = false;
        self.diff_selection = None;
        self.remember_view();
        self.open_pane(crate::panes::Kind::Tool("Document"), self.active, cx);
        cx.notify();
    }
    fn capture_document_selection(&mut self, cx: &mut Context<Self>) {
        let Some(document) = self.documents.get_mut(&(self.active, self.markdown_file)) else {
            return;
        };
        let source = document.source.read(cx).value().to_string();
        let selection = match document.mode {
            Mode::Preview => document.preview.selected_anchor(cx),
            Mode::Edit => {
                let bytes = document.source.read(cx).selected_range();
                flows::Anchor::markdown(&source, bytes)
            }
            Mode::Changes if document.rendered_changes => {
                let old = document.before.selected_anchor(cx);
                let new = document.preview.selected_anchor(cx);
                let old_span = match &old {
                    Some(flows::Anchor::Markdown { lines, .. }) => Some(lines.clone()),
                    _ => None,
                };
                let new_span = match &new {
                    Some(flows::Anchor::Markdown { lines, .. }) => Some(lines.clone()),
                    _ => None,
                };
                if old_span.is_none() && new_span.is_none() {
                    None
                } else {
                    Some(flows::Anchor::Diff {
                        old: old_span,
                        new: new_span,
                        quote: [
                            old.map(|a| format!("- {}", a.quote())),
                            new.map(|a| format!("+ {}", a.quote())),
                        ]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join("\n"),
                    })
                }
            }
            _ => None,
        };
        if selection.is_some() {
            document.selection = selection;
            cx.notify();
        }
    }
    fn document_comment_card(&self, index: usize, source: &str, cx: &mut Context<Self>) -> Div {
        let note = &self.comments[index];
        let recipient = &self.sessions[note.session];
        let stale = note.anchor.outdated(source);
        let mut card = col()
            .p_3()
            .gap_2()
            .bg(rgb(0x20232d))
            .border_l_2()
            .border_color(rgb(if stale { AMBER } else { ACCENT }))
            .child(
                row()
                    .gap_2()
                    .child(muted(format!("You · {}", note.anchor.label())))
                    .child(div().flex_1())
                    .child(pill(match note.delivery {
                        Delivery::Draft => "Draft",
                        Delivery::Queued => "Queued",
                        Delivery::Sent => "Sent",
                    })),
            )
            .child(
                div()
                    .id(("document-comment-context", index))
                    .max_h(px(100.))
                    .overflow_y_scroll()
                    .font_family("JetBrains Mono")
                    .text_size(px(11.))
                    .text_color(rgb(DIM))
                    .child(note.anchor.quote().to_owned()),
            )
            .child(gpui_kit::component::text::TextView::markdown(
                ("document-comment", index),
                note.body.clone(),
            ))
            .child(
                muted(format!("To {} · {}", recipient.agent, recipient.title)).text_size(px(10.)),
            );
        if stale {
            card = card.child(
                div()
                    .text_color(rgb(AMBER))
                    .child("The selected passage changed. Original context is kept."),
            );
        }
        if note.delivery == Delivery::Draft {
            card = card.child(
                row()
                    .gap_2()
                    .child(
                        Button::new(("send-document-comment", index))
                            .small()
                            .label("Send feedback")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.comments[index].delivery = Delivery::Queued;
                                let target = this.comments[index].session;
                                if matches!(this.sessions[target].status, "Idle" | "Needs you")
                                    && matches!(
                                        this.sessions[target].agent,
                                        "Claude Code" | "Copilot"
                                    )
                                {
                                    this.deliver_feedback_to(target, cx);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(("discard-document-comment", index))
                            .ghost()
                            .small()
                            .label("Discard")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.comments.remove(index);
                                cx.notify();
                            })),
                    ),
            );
        } else {
            card = card.child(
                row()
                    .gap_2()
                    .child(
                        Button::new(("open-feedback-agent", index))
                            .ghost()
                            .small()
                            .label("Open agent terminal")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let target = this.comments[index].session;
                                this.select(target, cx);
                                this.terminal_tab = "Agent";
                                this.review = false;
                                cx.notify();
                            })),
                    )
                    .when(note.delivery == Delivery::Queued, |d| {
                        d.child(
                            Button::new(("send-document-now", index))
                                .small()
                                .label("Send now")
                                .disabled(matches!(recipient.status, "Disconnected" | "Exited"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.deliver_feedback_to(this.comments[index].session, cx)
                                })),
                        )
                    }),
            );
        }
        card
    }
    fn document_annotations(
        &self,
        file: usize,
        source: &str,
        inline_composer: bool,
        cx: &mut Context<Self>,
    ) -> Vec<(usize, AnyElement)> {
        let mut notes = vec![];
        for (index, note) in self
            .comments
            .iter()
            .enumerate()
            .filter(|(_, note)| note.owner == self.active && note.file == file)
        {
            let offset = match &note.anchor {
                flows::Anchor::Markdown { bytes, .. } => bytes.end,
                _ => source.len(),
            };
            notes.push((
                offset,
                self.document_comment_card(index, source, cx)
                    .into_any_element(),
            ));
        }
        if inline_composer && self.comment_open && self.selected_file == file {
            let offset = match &self.comment_anchor {
                flows::Anchor::Markdown { bytes, .. } => bytes.end,
                _ => source.len(),
            };
            notes.push((offset, self.diff_composer(cx).into_any_element()));
        }
        notes.sort_by_key(|(offset, _)| *offset);
        notes
    }
    pub(super) fn document_pane(&self, cx: &mut Context<Self>) -> Div {
        let file = self.markdown_file;
        let Some(document) = self.documents.get(&(self.active, file)) else {
            return col()
                .p_4()
                .gap_3()
                .child("Open a Markdown file from this session’s Files pane.");
        };
        let source = document.source.read(cx).value().to_string();
        let (repo, path) = flows::FILES[file];
        let mut picker = row().px_3().py_2().gap_2();
        for other in 2..5usize {
            picker = picker.child(
                Button::new(("markdown-file", other))
                    .ghost()
                    .small()
                    .label(flows::FILES[other].1.rsplit('/').next().unwrap())
                    .selected(file == other)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_document(other, window, cx)
                    })),
            );
        }
        let mut modes = row()
            .px_3()
            .py_2()
            .gap_2()
            .border_b_1()
            .border_color(rgb(LINE));
        for (label, mode) in [
            ("Preview", Mode::Preview),
            ("Edit", Mode::Edit),
            ("Changes", Mode::Changes),
        ] {
            modes = modes.child(
                Button::new(SharedString::from(format!("markdown-mode-{label}")))
                    .ghost()
                    .small()
                    .label(label)
                    .selected(document.mode == mode)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(document) =
                            this.documents.get_mut(&(this.active, this.markdown_file))
                        {
                            let source = document.source.read(cx).value().to_string();
                            document.preview = crate::rich::Document::new(&source);
                            document.mode = mode;
                            document.selection = None;
                        }
                        this.comment_open = false;
                        this.diff_selection = None;
                        cx.notify();
                    })),
            );
        }
        modes = modes.child(div().flex_1()).child(
            Button::new("comment-markdown-selection")
                .small()
                .label("Comment on selection")
                .disabled(document.selection.is_none())
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(anchor) = this
                        .documents
                        .get(&(this.active, this.markdown_file))
                        .and_then(|doc| doc.selection.clone())
                    {
                        this.comment_anchor = anchor;
                        this.selected_file = this.markdown_file;
                        this.comment_open = true;
                        cx.notify();
                    }
                })),
        );
        let recipient = &self.sessions[document.recipient];
        let mut recipients = col().px_3().py_2().gap_2().child(
            row()
                .gap_2()
                .child(muted("Feedback to").text_size(px(10.)))
                .child(
                    Button::new("markdown-recipient-picker")
                        .ghost()
                        .small()
                        .icon(IconName::ChevronDown)
                        .label(format!("{} · {}", recipient.agent, recipient.title))
                        .on_click(cx.listener(|this, _, _, cx| {
                            let document = this
                                .documents
                                .get_mut(&(this.active, this.markdown_file))
                                .unwrap();
                            document.choosing_recipient = !document.choosing_recipient;
                            cx.notify();
                        })),
                ),
        );
        if document.choosing_recipient {
            for (target, session) in self.sessions.iter().enumerate().filter(|(_, session)| {
                session.channel == self.sessions[self.active].channel && !session.archived
            }) {
                recipients = recipients.child(
                    Button::new(("markdown-recipient", target))
                        .ghost()
                        .small()
                        .justify_start()
                        .label(format!("{} · {}", session.agent, session.title))
                        .selected(document.recipient == target)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(document) =
                                this.documents.get_mut(&(this.active, this.markdown_file))
                            {
                                document.recipient = target;
                                document.choosing_recipient = false;
                            }
                            cx.notify();
                        })),
                );
            }
        }
        let content = match document.mode {
            Mode::Preview => {
                let notes = self.document_annotations(file, &source, true, cx);
                document
                    .preview
                    .view_annotated("markdown-preview", cx, notes)
                    .into_any_element()
            }
            Mode::Edit => {
                let mut editor = col().gap_3().child(
                    div()
                        .id("markdown-editor")
                        .h(px(500.))
                        .child(Editor::new(&document.source).h_full())
                        .test_support(),
                );
                for (_, note) in self.document_annotations(file, &source, false, cx) {
                    editor = editor.child(note);
                }
                editor.into_any_element()
            }
            Mode::Changes => {
                let mut changes = col().gap_3().child(
                    row()
                        .gap_2()
                        .child(
                            Button::new("markdown-changes-source")
                                .small()
                                .label("Source diff")
                                .selected(!document.rendered_changes)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.documents
                                        .get_mut(&(this.active, this.markdown_file))
                                        .unwrap()
                                        .rendered_changes = false;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("markdown-changes-rendered")
                                .small()
                                .label("Rendered changes")
                                .selected(document.rendered_changes)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.documents
                                        .get_mut(&(this.active, this.markdown_file))
                                        .unwrap()
                                        .rendered_changes = true;
                                    cx.notify();
                                })),
                        ),
                );
                if document.rendered_changes {
                    changes = changes
                        .child(
                            col()
                                .p_3()
                                .bg(rgb(0x302020))
                                .gap_3()
                                .child("Before · original revision")
                                .child(document.before.view("markdown-before", cx)),
                        )
                        .child(
                            col()
                                .p_3()
                                .bg(rgb(0x1a2923))
                                .gap_3()
                                .child("After · current revision")
                                .child(document.preview.view("markdown-after", cx)),
                        );
                } else {
                    changes = changes.child(muted("Drag lines or Shift-click to comment across additions and deletions.").text_size(px(10.)))
                        .child(div().id("markdown-diff-scroll").overflow_x_scroll().child(self.review_rows(file, flows::document_diff(&document.baseline, &source), cx)));
                }
                for (_, note) in self.document_annotations(file, &source, false, cx) {
                    changes = changes.child(note);
                }
                changes.into_any_element()
            }
        };
        col()
            .size_full()
            .min_h_0()
            .child(
                col()
                    .px_3()
                    .pt_3()
                    .gap_1()
                    .child(path)
                    .child(muted(repo).text_size(px(10.))),
            )
            .child(picker)
            .child(modes)
            .child(recipients)
            .when(document.mode != Mode::Preview && self.comment_open && self.selected_file == file, |d| {
                d.child(self.diff_composer(cx).flex_shrink_0())
            })
            .child(
                div()
                    .id("markdown-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_4()
                    .child(content)
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.capture_document_selection(cx);
                            let entity = cx.entity().downgrade();
                            cx.defer(move |cx| {
                                let _ = entity
                                    .update(cx, |this, cx| this.capture_document_selection(cx));
                            });
                        }),
                    )
                    .on_key_up(cx.listener(|this, _, _, cx| {
                        this.capture_document_selection(cx);
                        let entity = cx.entity().downgrade();
                        cx.defer(move |cx| {
                            let _ =
                                entity.update(cx, |this, cx| this.capture_document_selection(cx));
                        });
                    }))
                    .test_support(),
            )
    }
    pub(super) fn document_recipient(&self, file: usize) -> usize {
        self.documents
            .get(&(self.active, file))
            .map(|document| document.recipient)
            .unwrap_or(self.active)
    }
}
