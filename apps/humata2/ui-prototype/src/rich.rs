//! Shared Markdown and Mermaid presentation for prototype documents and comments.
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::text::{SelectionFormat, TextView, TextViewState};
use gpui_kit::{prelude::*, *};
use std::sync::Arc;

pub(super) const SAMPLE: &str = r#"# Billing v2

A **reversible ledger migration**, reviewed by you and the agent. See the [acceptance criteria](#acceptance-criteria).

> Keep existing invoices readable throughout the rollout.

## Acceptance criteria

- [x] Define integer postings
- [ ] Backfill existing invoices
- [ ] Verify the rollback on staging

| Change | Owner | State |
| --- | --- | --- |
| Schema | Claude Code | In review |
| Backfill | Codex | Blocked |

```rust
fn balanced(debit: i64, credit: i64) -> bool {
    debit == credit
}
```

## Work and execution

```mermaid
flowchart LR
    M[Milestone] --> S[Step]
    S --> A[Agent session]
    T[Standalone task] --> A
    A --> R[Review]
```

## Review delivery

```mermaid
sequenceDiagram
    participant User
    participant Humata
    participant Agent
    User->>Humata: Add draft comment
    User->>Humata: Send feedback
    Humata->>Agent: Paste when the turn finishes
    Agent-->>User: Address the feedback
```

## Recovery

```mermaid
stateDiagram-v2
    Running --> Hidden: Close
    Hidden --> Running: Reopen
    Running --> Exited: Stop
    Exited --> Running: Resume
```

---

A very long identifier for wrapping: `ledger_entries_account_id_currency_created_at_idempotency_key_index`.
"#;

#[derive(Debug)]
pub(super) enum DiagramError {
    InvalidSyntax {
        source: mermaid_rs_renderer::ParseError,
    },
}
impl std::fmt::Display for DiagramError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSyntax { source } => write!(
                f,
                "Diagram could not render: {source}. Edit the Mermaid source and preview again."
            ),
        }
    }
}
impl std::error::Error for DiagramError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidSyntax { source } => Some(source),
        }
    }
}

enum Section {
    Markdown {
        source: String,
        bytes: std::ops::Range<usize>,
        state: std::cell::RefCell<Option<Entity<TextViewState>>>,
    },
    Diagram {
        source: String,
        bytes: std::ops::Range<usize>,
        image: Result<Arc<Image>, DiagramError>,
        raster: std::cell::RefCell<Option<Arc<RenderImage>>>,
    },
}
pub(super) struct Document {
    source: String,
    sections: Vec<Section>,
}
impl Document {
    pub(super) fn new(source: &str) -> Self {
        let mut sections = vec![];
        if let Ok(markdown::mdast::Node::Root(root)) =
            markdown::to_mdast(source, &markdown::ParseOptions::gfm())
        {
            for node in root.children {
                let Some(position) = node.position() else {
                    continue;
                };
                let bytes = position.start.offset..position.end.offset;
                let Some(text) = source.get(bytes.clone()) else {
                    continue;
                };
                if let markdown::mdast::Node::Code(code) = &node {
                    if code.lang.as_deref() == Some("mermaid") {
                        let image = render_diagram(&code.value).map(|svg| {
                            Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes()))
                        });
                        sections.push(Section::Diagram {
                            source: code.value.clone(),
                            bytes,
                            image,
                            raster: std::cell::RefCell::new(None),
                        });
                        continue;
                    }
                }
                sections.push(Section::Markdown {
                    source: text.into(),
                    bytes,
                    state: std::cell::RefCell::new(None),
                });
            }
        }
        if sections.is_empty() && !source.is_empty() {
            sections.push(Section::Markdown {
                source: source.into(),
                bytes: 0..source.len(),
                state: std::cell::RefCell::new(None),
            });
        }
        Self {
            source: source.into(),
            sections,
        }
    }
    pub(super) fn selected_anchor(&self, cx: &App) -> Option<crate::flows::Anchor> {
        let mut selected = vec![];
        for section in &self.sections {
            if let Section::Markdown { bytes, state, .. } = section {
                if let Some(state) = state.borrow().as_ref() {
                    let state = state.read(cx);
                    if let Some(range) = state
                        .selected_source_range()
                        .filter(|range| !range.is_empty())
                    {
                        selected.push(bytes.start + range.start..bytes.start + range.end);
                    }
                }
            }
        }
        let start = selected.first()?.start;
        let end = selected.last()?.end;
        crate::flows::Anchor::markdown(&self.source, start..end)
    }
    pub(super) fn view(&self, id: impl Into<ElementId>, cx: &mut App) -> Stateful<Div> {
        self.view_annotated(id, cx, vec![])
    }
    pub(super) fn view_annotated(
        &self,
        id: impl Into<ElementId>,
        cx: &mut App,
        notes: Vec<(usize, AnyElement)>,
    ) -> Stateful<Div> {
        let mut notes: Vec<_> = notes.into_iter().map(Some).collect();
        let mut document = div().id(id).flex().flex_col().gap_4();
        for (index, section) in self.sections.iter().enumerate() {
            match section {
                Section::Markdown { source, state, .. } => {
                    if state.borrow().is_none() {
                        *state.borrow_mut() = Some(cx.new(|cx| {
                            TextViewState::markdown(source, cx)
                                .selection_format(SelectionFormat::Plain)
                        }));
                    }
                    let state = state.borrow().as_ref().unwrap().clone();
                    document = document.child(
                        div()
                            .id(("markdown-block", index))
                            .child(TextView::new(&state).selectable(true))
                            .test_support(),
                    );
                }
                Section::Diagram {
                    source,
                    image,
                    raster,
                    ..
                } => {
                    let mut diagram = div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_3()
                        .rounded_md()
                        .bg(rgb(0x202126));
                    match image {
                        Ok(image) => {
                            if raster.borrow().is_none() {
                                match image.to_image_data(cx.svg_renderer()) {
                                    Ok(rendered) => *raster.borrow_mut() = Some(rendered),
                                    Err(error) => diagram = diagram.child(format!("Could not display diagram: {error}. Edit the source and preview again.")),
                                }
                            }
                            if let Some(image) = raster.borrow().as_ref() {
                                diagram = diagram.child(
                                    img(image.clone())
                                        .w_full()
                                        .h(px(260.))
                                        .object_fit(ObjectFit::Contain),
                                );
                            }
                        }
                        Err(error) => {
                            diagram = diagram
                                .child(div().text_color(rgb(0xe3b87c)).child(error.to_string()))
                        }
                    }
                    document = document.child(diagram.when(image.is_err(), |d| {
                        d.child(TextView::markdown(
                            ("mermaid-source", index),
                            format!("```mermaid\n{source}\n```"),
                        ))
                    }));
                }
            }
            let end = match section {
                Section::Markdown { bytes, .. } | Section::Diagram { bytes, .. } => bytes.end,
            };
            for note in &mut notes {
                if note.as_ref().is_some_and(|(offset, _)| *offset <= end) {
                    document = document.child(note.take().unwrap().1);
                }
            }
        }
        for (_, note) in notes.into_iter().flatten() {
            document = document.child(note);
        }
        document
    }
}
fn render_diagram(source: &str) -> Result<String, DiagramError> {
    let parsed = mermaid_rs_renderer::parse_mermaid_strict(source)
        .map_err(|source| DiagramError::InvalidSyntax { source })?;
    let mut theme = mermaid_rs_renderer::Theme::dark();
    theme.font_family = "IBM Plex Sans".into();
    let config = mermaid_rs_renderer::LayoutConfig::default();
    let layout = mermaid_rs_renderer::compute_layout(&parsed.graph, &theme, &config);
    Ok(mermaid_rs_renderer::render_svg(&layout, &theme, &config))
}
#[cfg(test)]
mod tests {
    use super::*;
    use ::std::prelude::v1::test;
    #[test]
    fn markdown_fences_keep_code_and_render_mermaid_diagrams() {
        let document = Document::new(SAMPLE);
        let diagrams: Vec<_> = document
            .sections
            .iter()
            .filter_map(|s| match s {
                Section::Diagram { image, .. } => Some(image),
                _ => None,
            })
            .collect();
        assert_eq!(diagrams.len(), 3);
        assert!(diagrams.iter().all(|image| image.is_ok()));
        assert!(matches!(
            render_diagram("this is not a diagram"),
            Err(DiagramError::InvalidSyntax { .. })
        ));
    }
}
