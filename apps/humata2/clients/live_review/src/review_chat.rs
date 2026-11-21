//! [`ReviewChatClient`] for the real daemon, over ReviewChatService (PR-22 to
//! PR-26, D59). The app reports the view it shows; the agent's changes come
//! back on the change feed, pushed as they happen, and nothing polls.

use humata2_clients::comments::{Anchor, CommentId, Side};
use humata2_clients::model::SessionId;
use humata2_clients::review_chat::{
    AgentSwitches, ChangeEvents, FocusRequest, Highlight, LineSpan, Note, Place, ReviewChange,
    ReviewChangeKind, ReviewChatClient, ReviewContext, ReviewView, ViewedFile,
};
use humata2_clients::views::PlanGroup;
use humata2_clients::Reply;
use humata2_clients_live_core::{LiveConnection, Wire, fresh_key, map};
use crate::humata2_grpc::humata::v1 as pb;
use crate::humata2_grpc::humata::v1::review_chat_service_client::ReviewChatServiceClient;
use tokio::sync::mpsc;
use tonic::Streaming;

use crate::views;

const WHAT: &str = "The review's chat";

/// The review chat on the real daemon. Cheap to clone.
#[derive(Clone)]
pub struct LiveReviewChat {
    shared: LiveConnection,
}

impl LiveReviewChat {
    /// The review chat over `conn`'s connection.
    #[must_use]
    pub fn new(conn: impl AsRef<LiveConnection>) -> Self {
        Self {
            shared: conn.as_ref().clone(),
        }
    }
}

fn client(shared: &LiveConnection) -> ReviewChatServiceClient<Wire> {
    ReviewChatServiceClient::new(shared.wire())
}

fn session_wire(session: &SessionId) -> Option<pb::SessionId> {
    Some(pb::SessionId {
        value: session.as_str().to_owned(),
    })
}

/// The side of a comparison the daemon names; unknown or unset is the new side.
fn side_of(value: i32) -> Side {
    match pb::DiffSide::try_from(value) {
        Ok(pb::DiffSide::Old) => Side::Old,
        _ => Side::New,
    }
}

fn side_to_wire(side: Side) -> i32 {
    match side {
        Side::Old => pb::DiffSide::Old as i32,
        Side::New => pb::DiffSide::New as i32,
    }
}

fn lines_of(wire: Option<&pb::LineRange>) -> Option<LineSpan> {
    wire.map(|lines| LineSpan {
        first: lines.first,
        last: lines.last,
    })
}

fn lines_to_wire(lines: Option<LineSpan>) -> Option<pb::LineRange> {
    lines.map(|lines| pb::LineRange {
        first: lines.first,
        last: lines.last,
    })
}

fn repo_path(repo: &str, path: &str) -> pb::RepoPath {
    pb::RepoPath {
        repo: map::parse_repo(repo),
        path: path.to_owned(),
    }
}

/// A file as `(repo, path)`; `None` when the daemon named no repository.
fn file_of(wire: Option<&pb::RepoPath>) -> Option<(String, String)> {
    let file = wire?;
    Some((file.repo.as_ref().map(map::repo_text)?, file.path.clone()))
}

fn highlight(wire: &pb::Highlight) -> Option<Highlight> {
    let (repo, path) = file_of(wire.file.as_ref())?;
    Some(Highlight {
        repo,
        path,
        lines: lines_of(wire.lines.as_ref()),
        side: side_of(wire.side),
        label: wire.label.clone(),
    })
}

fn highlight_to_wire(highlight: &Highlight) -> pb::Highlight {
    pb::Highlight {
        file: Some(repo_path(&highlight.repo, &highlight.path)),
        lines: lines_to_wire(highlight.lines),
        side: side_to_wire(highlight.side),
        label: highlight.label.clone(),
    }
}

fn place(wire: &pb::ReviewPlace) -> Option<Place> {
    let (repo, path) = file_of(wire.file.as_ref())?;
    Some(Place {
        repo,
        path,
        visible: lines_of(wire.visible.as_ref()),
        selection: lines_of(wire.selection.as_ref()),
        side: side_of(wire.side),
    })
}

fn place_to_wire(place: &Place) -> pb::ReviewPlace {
    pb::ReviewPlace {
        file: Some(repo_path(&place.repo, &place.path)),
        visible: lines_to_wire(place.visible),
        selection: lines_to_wire(place.selection),
        side: side_to_wire(place.side),
    }
}

/// The view as the daemon sends it back; an absent view is an empty one.
fn view(wire: Option<&pb::ReviewViewState>) -> ReviewView {
    let Some(wire) = wire else {
        return ReviewView::default();
    };
    ReviewView {
        targets: wire
            .spec
            .as_ref()
            .map(|spec| spec.repos.iter().filter_map(views::repo_diff_spec).collect())
            .unwrap_or_default(),
        open: wire.open.as_ref().and_then(place),
        order: wire.order.iter().filter_map(|path| file_of(Some(path))).collect(),
        groups: wire
            .groups
            .iter()
            .filter_map(|group| {
                Some(PlanGroup {
                    title: group.title.clone(),
                    note: group.note.clone(),
                    first: file_of(group.first.as_ref())?,
                })
            })
            .collect(),
        viewed: wire
            .viewed
            .iter()
            .filter_map(|viewed| {
                let (repo, path) = file_of(viewed.file.as_ref())?;
                Some(ViewedFile {
                    repo,
                    path,
                    blob_sha: viewed.blob_sha.clone(),
                })
            })
            .collect(),
        highlights: wire.highlights.iter().filter_map(highlight).collect(),
        stack: wire.stack.as_ref().map(views::stack),
        layer: (wire.stack.is_some() && !wire.layer.is_empty()).then(|| wire.layer.clone()),
    }
}

fn view_to_wire(view: &ReviewView) -> pb::ReviewViewState {
    pb::ReviewViewState {
        spec: humata2_clients_live_diff::spec_to_wire(&view.targets),
        open: view.open.as_ref().map(place_to_wire),
        order: view
            .order
            .iter()
            .map(|(repo, path)| repo_path(repo, path))
            .collect(),
        groups: view
            .groups
            .iter()
            .map(|group| pb::ReviewGroup {
                title: group.title.clone(),
                note: group.note.clone(),
                first: Some(repo_path(&group.first.0, &group.first.1)),
            })
            .collect(),
        viewed: view
            .viewed
            .iter()
            .map(|viewed| pb::ViewedFile {
                file: Some(repo_path(&viewed.repo, &viewed.path)),
                blob_sha: viewed.blob_sha.clone(),
            })
            .collect(),
        highlights: view.highlights.iter().map(highlight_to_wire).collect(),
        stack: view.stack.as_ref().map(views::stack_to_wire),
        layer: view.layer.clone().unwrap_or_default(),
        // The daemon keeps the plan's tiers itself (D802).
        importance: Vec::new(),
    }
}

/// A comment's anchor as the daemon keeps it, outdated when its lines changed
/// since (PR-26); `None` if the daemon named no file.
pub(crate) fn anchor(wire: &pb::CommentAnchor) -> Option<Anchor> {
    let (repo, path) = file_of(wire.file.as_ref())?;
    let lines = wire.lines.as_ref();
    Some(Anchor {
        repo,
        path,
        side: side_of(wire.side),
        first: lines.map_or(0, |lines| lines.first),
        last: lines.map_or(0, |lines| lines.last),
        quoted: wire.quoted.lines().map(str::to_owned).collect(),
        base: Some(wire.base.clone()).filter(|base| !base.is_empty()),
        detached: wire.detached,
        outdated: wire.outdated,
    })
}

/// A note the agent left, as the daemon sends a comment with the note
/// destination.
fn note(wire: &pb::Comment) -> Option<Note> {
    let id = wire.id.as_ref().map(|id| id.value.clone()).filter(|id| !id.is_empty())?;
    let anchor = anchor(wire.anchor.as_ref()?)?;
    Some(Note {
        id: CommentId::new(id),
        anchor,
        text: wire.text.clone(),
    })
}

fn switches_of(wire: Option<&pb::AgentSwitches>) -> AgentSwitches {
    // Nothing the daemon doesn't name is allowed.
    wire.map_or(
        AgentSwitches {
            focus: false,
            notes: false,
            order: false,
        },
        |switches| AgentSwitches {
            focus: switches.focus,
            notes: switches.notes,
            order: switches.order,
        },
    )
}

fn switches_to_wire(switches: AgentSwitches) -> pb::AgentSwitches {
    pb::AgentSwitches {
        focus: switches.focus,
        notes: switches.notes,
        order: switches.order,
    }
}

fn change_kind(wire: &pb::review_change::Change) -> Option<ReviewChangeKind> {
    use pb::review_change::Change;
    match wire {
        Change::Focus(place_wire) => place(place_wire).map(ReviewChangeKind::Focus),
        Change::Note(comment) => note(comment).map(ReviewChangeKind::Note),
        Change::NoteDropped(id) => Some(ReviewChangeKind::NoteDropped(CommentId::new(id.value.clone()))),
        Change::Plan(plan) => Some(ReviewChangeKind::Plan(views::plan(plan))),
        // D800: empty is Whole stack.
        Change::Layer(title) => Some(ReviewChangeKind::Layer(
            (!title.is_empty()).then(|| title.clone()),
        )),
    }
}

/// One change to the view as the feed or an answer carries it.
fn change(wire: &pb::ReviewChange) -> ReviewChange {
    ReviewChange {
        undo_id: wire.undo_id.clone(),
        seq: wire.seq,
        tool: wire.tool.clone(),
        summary: wire.summary.clone(),
        change: wire.change.as_ref().and_then(change_kind),
        highlights: wire.highlights.iter().filter_map(highlight).collect(),
        undid: wire.undid.clone(),
        at_unix_ms: wire
            .at
            .as_ref()
            .map_or(0, |at| map::unix_ms(at.seconds, at.nanos)),
    }
}

/// Maps each change until the feed ends or nobody listens.
async fn forward(mut stream: Streaming<pb::ReviewChange>, sender: mpsc::UnboundedSender<ReviewChange>) {
    loop {
        let next = tokio::select! {
            () = sender.closed() => return,
            next = stream.message() => next,
        };
        match next {
            Ok(Some(wire)) => {
                if sender.send(change(&wire)).is_err() {
                    return;
                }
            }
            Ok(None) | Err(_) => return,
        }
    }
}

impl ReviewChatClient for LiveReviewChat {
    fn context(&self, session: &SessionId) -> Reply<ReviewContext> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move {
            let answer = client(&shared)
                .get_review_context(pb::GetReviewContextRequest {
                    session: session_wire(&session),
                })
                .await
                .map_err(|status| shared.error(&status, WHAT))?
                .into_inner();
            Ok(ReviewContext {
                view: view(answer.view.as_ref()),
                selected_text: answer.selected_text.clone(),
                notes: answer.notes.iter().filter_map(note).collect(),
                switches: switches_of(answer.switches.as_ref()),
            })
        })
    }

    fn report_view(&self, session: &SessionId, view: ReviewView) -> Reply<()> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move {
            client(&shared)
                .report_review_view(pb::ReportReviewViewRequest {
                    session: session_wire(&session),
                    view: Some(view_to_wire(&view)),
                })
                .await
                .map_err(|status| shared.error(&status, WHAT))?;
            Ok(())
        })
    }

    fn focus(&self, session: &SessionId, request: FocusRequest) -> Reply<ReviewChange> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move {
            let answer = client(&shared)
                .focus_review(pb::FocusReviewRequest {
                    idempotency_key: fresh_key(),
                    session: session_wire(&session),
                    file: Some(repo_path(&request.repo, &request.path)),
                    lines: lines_to_wire(request.lines),
                    side: side_to_wire(request.side),
                    highlights: request.highlights.iter().map(highlight_to_wire).collect(),
                    layer: request.layer.clone().unwrap_or_default(),
                    whole_stack: request.whole_stack,
                })
                .await
                .map_err(|status| shared.error(&status, WHAT))?
                .into_inner();
            Ok(change(&answer))
        })
    }

    fn undo(&self, session: &SessionId, undo_id: &str) -> Reply<ReviewChange> {
        let shared = self.shared.clone();
        let session = session.clone();
        let undo_id = undo_id.to_owned();
        Box::pin(async move {
            let answer = client(&shared)
                .undo_review_change(pb::UndoReviewChangeRequest {
                    session: session_wire(&session),
                    undo_id,
                })
                .await
                .map_err(|status| shared.error(&status, WHAT))?
                .into_inner();
            Ok(change(&answer))
        })
    }

    fn watch_changes(&self, session: &SessionId, after_seq: u64) -> Reply<ChangeEvents> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move {
            // Open before this answers: the stream is the call's own.
            let stream = client(&shared)
                .watch_review_changes(pb::WatchReviewChangesRequest {
                    session: session_wire(&session),
                    after_seq,
                })
                .await
                .map_err(|status| shared.error(&status, WHAT))?
                .into_inner();
            let (sender, receiver) = mpsc::unbounded_channel();
            shared.spawn(forward(stream, sender));
            Ok(receiver)
        })
    }

    fn set_switches(&self, session: &SessionId, switches: AgentSwitches) -> Reply<AgentSwitches> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move {
            let answer = client(&shared)
                .set_agent_switches(pb::SetAgentSwitchesRequest {
                    session: session_wire(&session),
                    switches: Some(switches_to_wire(switches)),
                })
                .await
                .map_err(|status| shared.error(&status, WHAT))?
                .into_inner();
            Ok(switches_of(Some(&answer)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place_wire(path: &str) -> pb::ReviewPlace {
        pb::ReviewPlace {
            file: Some(repo_path("acme/api", path)),
            visible: Some(pb::LineRange { first: 120, last: 160 }),
            selection: None,
            side: pb::DiffSide::New as i32,
        }
    }

    #[test]
    fn a_focus_change_keeps_its_place_and_its_undo() {
        let wire = pb::ReviewChange {
            undo_id: "u-4".to_owned(),
            seq: 4,
            tool: "review_focus".to_owned(),
            summary: "refund.rs:140".to_owned(),
            change: Some(pb::review_change::Change::Focus(place_wire("src/refund.rs"))),
            ..pb::ReviewChange::default()
        };
        let mapped = change(&wire);
        assert_eq!(mapped.undo_id, "u-4");
        assert_eq!(mapped.seq, 4);
        let Some(ReviewChangeKind::Focus(place)) = mapped.change else {
            panic!("a focus change maps to a place");
        };
        assert_eq!(place.path, "src/refund.rs");
        assert_eq!(place.visible, Some(LineSpan { first: 120, last: 160 }));
    }

    #[test]
    fn a_diff_change_has_no_undo_and_no_change() {
        let wire = pb::ReviewChange {
            tool: "diff".to_owned(),
            summary: "moved".to_owned(),
            ..pb::ReviewChange::default()
        };
        let mapped = change(&wire);
        assert!(mapped.undo_id.is_empty());
        assert_eq!(mapped.change, None);
    }

    #[test]
    fn a_note_keeps_its_anchor_text_and_whether_it_is_outdated() {
        let wire = pb::Comment {
            id: Some(pb::CommentId { value: "acme/api/c-3".to_owned() }),
            text: "Rounds the wrong way".to_owned(),
            anchor: Some(pb::CommentAnchor {
                file: Some(repo_path("acme/api", "src/refund.rs")),
                lines: Some(pb::LineRange { first: 140, last: 141 }),
                side: pb::DiffSide::New as i32,
                quoted: "let a = 1;\nlet b = 2;".to_owned(),
                outdated: true,
                ..pb::CommentAnchor::default()
            }),
            ..pb::Comment::default()
        };
        let mapped = note(&wire).expect("a note with an id and a file maps");
        assert_eq!(mapped.id, CommentId::new("acme/api/c-3"));
        assert_eq!(mapped.text, "Rounds the wrong way");
        assert!(mapped.anchor.outdated);
        assert_eq!(mapped.anchor.first, 140);
        assert_eq!(mapped.anchor.last, 141);
        assert_eq!(mapped.anchor.quoted, vec!["let a = 1;".to_owned(), "let b = 2;".to_owned()]);
        assert_eq!(mapped.anchor.base, None);
    }

    #[test]
    fn switches_the_daemon_leaves_out_are_all_off() {
        assert_eq!(
            switches_of(None),
            AgentSwitches {
                focus: false,
                notes: false,
                order: false,
            }
        );
    }

    #[test]
    fn a_view_survives_the_round_trip_to_the_daemon_and_back() {
        use humata2_clients::model::RepoDiffSpec;
        let original = ReviewView {
            targets: vec![RepoDiffSpec {
                repo: "acme/api".to_owned(),
                base: humata2_clients::model::RepoBase::Branch("main".to_owned()),
                options: humata2_clients::model::RepoDiffOptions::default(),
            }],
            open: Some(Place {
                repo: "acme/api".to_owned(),
                path: "src/refund.rs".to_owned(),
                visible: Some(LineSpan { first: 1, last: 9 }),
                selection: None,
                side: Side::New,
            }),
            order: vec![("acme/api".to_owned(), "src/refund.rs".to_owned())],
            groups: vec![PlanGroup {
                title: "Money".to_owned(),
                note: String::new(),
                first: ("acme/api".to_owned(), "src/refund.rs".to_owned()),
            }],
            viewed: vec![ViewedFile {
                repo: "acme/api".to_owned(),
                path: "README.md".to_owned(),
                blob_sha: "abc".to_owned(),
            }],
            highlights: vec![Highlight {
                repo: "acme/api".to_owned(),
                path: "src/refund.rs".to_owned(),
                lines: Some(LineSpan { first: 2, last: 3 }),
                side: Side::Old,
                label: "here".to_owned(),
            }],
            stack: None,
            layer: None,
        };
        assert_eq!(view(Some(&view_to_wire(&original))), original);
    }

    #[test]
    fn a_stack_and_its_layer_survive_the_round_trip() {
        use humata2_clients::model::{RepoBase, RepoDiffSpec};
        use humata2_clients::views::{LayerRepoView, LayerView, StackView};
        let spec = |base| RepoDiffSpec {
            repo: "acme/api".to_owned(),
            base,
            options: humata2_clients::model::RepoDiffOptions::default(),
        };
        let range = RepoBase::Range {
            base: "m".to_owned(),
            head: "x1".to_owned(),
        };
        let overlay = RepoBase::Overlay {
            base: "x1".to_owned(),
            paths: vec!["src".to_owned()],
        };
        let part = |base: RepoBase, pr: &str| LayerRepoView {
            repo: "acme/api".to_owned(),
            spec: spec(base),
            pull_request: pr.to_owned(),
            files: 2,
            drift: None,
        };
        let layers = vec![
            LayerView {
                title: "one".to_owned(),
                purpose: "first".to_owned(),
                repos: vec![part(range.clone(), "acme/api#1")],
                order: vec![("acme/api".to_owned(), "a.rs".to_owned())],
                ..LayerView::default()
            },
            LayerView {
                title: "wip".to_owned(),
                repos: vec![part(overlay.clone(), "")],
                ..LayerView::default()
            },
        ];
        let stack = StackView {
            layers,
            whole: vec![spec(RepoBase::Overlay {
                base: "m".to_owned(),
                paths: vec!["src".to_owned()],
            })],
        };
        let original = ReviewView {
            stack: Some(stack),
            layer: Some("wip".to_owned()),
            ..ReviewView::default()
        };
        assert_eq!(view(Some(&view_to_wire(&original))), original);
        let whole = ReviewView {
            layer: None,
            ..original
        };
        assert_eq!(view(Some(&view_to_wire(&whole))).layer, None);
    }

    #[test]
    fn a_layer_change_with_no_title_is_the_whole_stack() {
        let wire = |title: &str| pb::ReviewChange {
            change: Some(pb::review_change::Change::Layer(title.to_owned())),
            ..pb::ReviewChange::default()
        };
        assert_eq!(change(&wire("")).change, Some(ReviewChangeKind::Layer(None)));
        assert_eq!(
            change(&wire("api")).change,
            Some(ReviewChangeKind::Layer(Some("api".to_owned())))
        );
    }
}
