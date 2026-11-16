//! An upload that lives only while a folder holds it. The upload streams
//! its progress from a transport. The folder attaches it, may detach it
//! partway, and may attach a new one later. Nothing here is a view.
//!
//! "Presentation" in this library means any child that lives only while
//! it's shown or attached: a sheet, an upload, a connection. What matters
//! is that the child's work must not outlive it. This file runs the same
//! upload twice, first as plain `Option<Upload>` with `if_let`, then in a
//! `PresentationState<Upload>` with `if_presented`. The tests show what
//! each does with three things:
//!
//! - Detaching. `Option` drops the state, but the upload's stream keeps
//!   running. Dismissing a presentation cancels the child's effects.
//! - A late frame from upload 1 once upload 2 is attached. With `Option`
//!   it lands on upload 2, because actions don't say which upload they
//!   came from. A presentation cancelled upload 1's work when it was
//!   dismissed, so nothing of it is left to land.
//! - Ending itself. With `if_let`, `Effect::dismiss` is ignored, so the
//!   finished upload stays attached. A presentation turns it into
//!   `PresentationAction::Dismiss`.
//!
//! A parent can fix the first two by hand, by cancelling the child's
//! effect ids when it drops the state. That means knowing every id the
//! child uses, now and later. A presentation does it for any child.
//!
//! Patterns: a non-UI child whose lifetime matters; what `Option` and
//! `if_let` leave to the parent; `PresentationState`, `if_presented` and
//! `Effect::dismiss` for a child that isn't a view.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tca::{
    BoxFuture, Effect, EventStream, PresentationAction, PresentationState, Reducer, TestStore,
    if_let, if_presented,
};
use tokio::sync::mpsc;

// The transport: each upload opens a stream of how many bytes the server
// has. The fake counts open streams and lets a test send a frame on any
// stream it opened, or close it.

#[derive(Default)]
struct FakeTransport {
    open: Arc<AtomicUsize>,
    /// Every stream opened, by the order it was opened in.
    streams: Mutex<Vec<Option<mpsc::UnboundedSender<Result<u64, Infallible>>>>>,
}

impl FakeTransport {
    fn open_streams(&self) -> usize {
        self.open.load(Ordering::SeqCst)
    }

    /// Sends `sent` on stream `n`; false if whoever read it is gone.
    fn frame(&self, n: usize, sent: u64) -> bool {
        match self.streams.lock().unwrap().get(n) {
            Some(Some(stream)) => stream.send(Ok(sent)).is_ok(),
            _ => false,
        }
    }

    /// Ends stream `n`, as when the server has the whole file.
    fn close(&self, n: usize) {
        if let Some(stream) = self.streams.lock().unwrap().get_mut(n) {
            stream.take();
        }
    }

    fn upload(&self) -> BoxFuture<Result<Frames, Infallible>> {
        let (send, frames) = mpsc::unbounded_channel();
        self.streams.lock().unwrap().push(Some(send));
        self.open.fetch_add(1, Ordering::SeqCst);
        let frames = Frames {
            frames,
            open: self.open.clone(),
        };
        Box::pin(async move { Ok(frames) })
    }
}

/// One stream, counted as open until it's dropped.
struct Frames {
    frames: mpsc::UnboundedReceiver<Result<u64, Infallible>>,
    open: Arc<AtomicUsize>,
}

impl Drop for Frames {
    fn drop(&mut self) {
        self.open.fetch_sub(1, Ordering::SeqCst);
    }
}

impl EventStream for Frames {
    type Item = u64;
    type Error = Infallible;

    fn next(&mut self) -> Pin<Box<dyn Future<Output = Option<Result<u64, Infallible>>> + Send + '_>> {
        Box::pin(self.frames.recv())
    }
}

// The upload: the child both folders use, unchanged.

#[derive(Clone, Debug, Default, PartialEq)]
struct Upload {
    sent: u64,
}

#[derive(Clone, Debug, PartialEq)]
enum UploadAction {
    Start,
    Progress(u64),
    Finished,
}

/// Not cancelling in flight: the upload can't know whether another one,
/// held somewhere else, runs under the same id, and cancelling that one
/// would be wrong.
const STREAM: &str = "stream";

struct Uploading {
    transport: Arc<FakeTransport>,
}

impl Reducer for Uploading {
    type State = Upload;
    type Action = UploadAction;

    fn reduce(&self, upload: &mut Upload, action: UploadAction) -> Effect<UploadAction> {
        match action {
            UploadAction::Start => Effect::stream(
                self.transport.upload(),
                UploadAction::Progress,
                |_| UploadAction::Finished,
            )
            .cancellable(STREAM, false),
            UploadAction::Progress(sent) => {
                upload.sent = sent;
                Effect::none()
            }
            // Done: the upload ends itself, whoever holds it.
            UploadAction::Finished => Effect::dismiss(),
        }
    }
}

// The folder with plain optional state.

#[derive(Clone, Debug, Default, PartialEq)]
struct PlainFolder {
    upload: Option<Upload>,
    finished: u32,
}

#[derive(Debug, PartialEq)]
enum PlainAction {
    Attach,
    Detach,
    Upload(UploadAction),
}

struct PlainFolding {
    uploading: Uploading,
}

impl Reducer for PlainFolding {
    type State = PlainFolder;
    type Action = PlainAction;

    fn reduce(&self, folder: &mut PlainFolder, action: PlainAction) -> Effect<PlainAction> {
        match action {
            PlainAction::Attach => {
                folder.upload = Some(Upload::default());
                if_let(&self.uploading, &mut folder.upload, UploadAction::Start)
                    .map(PlainAction::Upload)
            }
            // What a parent writes first: drop the state. The tests show
            // what that leaves behind.
            PlainAction::Detach => {
                folder.upload = None;
                Effect::none()
            }
            PlainAction::Upload(action) => {
                if action == UploadAction::Finished {
                    folder.finished += 1;
                }
                if_let(&self.uploading, &mut folder.upload, action).map(PlainAction::Upload)
            }
        }
    }
}

// The folder with a presentation.

#[derive(Clone, Debug, Default, PartialEq)]
struct Folder {
    upload: PresentationState<Upload>,
    finished: u32,
}

#[derive(Debug, PartialEq)]
enum FolderAction {
    Attach,
    /// `Dismiss` detaches; `Presented` is for the upload.
    Upload(PresentationAction<UploadAction>),
}

struct Folding {
    uploading: Uploading,
}

impl Reducer for Folding {
    type State = Folder;
    type Action = FolderAction;

    fn reduce(&self, folder: &mut Folder, action: FolderAction) -> Effect<FolderAction> {
        match action {
            FolderAction::Attach => {
                // Ends the one attached, if any, before the next: `present`
                // alone would leave the old one's work running.
                let ended = folder.upload.dismiss();
                folder.upload.present(Upload::default());
                let started = if_presented(
                    &self.uploading,
                    &mut folder.upload,
                    PresentationAction::Presented(UploadAction::Start),
                );
                ended.and(started.map(FolderAction::Upload))
            }
            FolderAction::Upload(action) => {
                let finished = action == PresentationAction::Presented(UploadAction::Finished);
                let effect = if_presented(&self.uploading, &mut folder.upload, action)
                    .map(FolderAction::Upload);
                if finished {
                    folder.finished += 1;
                }
                effect
            }
        }
    }
}

fn plain() -> (TestStore<PlainFolding>, Arc<FakeTransport>) {
    let transport = Arc::new(FakeTransport::default());
    let reducer = PlainFolding {
        uploading: Uploading {
            transport: transport.clone(),
        },
    };
    (TestStore::new(PlainFolder::default(), reducer), transport)
}

fn presented() -> (TestStore<Folding>, Arc<FakeTransport>) {
    let transport = Arc::new(FakeTransport::default());
    let reducer = Folding {
        uploading: Uploading {
            transport: transport.clone(),
        },
    };
    (TestStore::new(Folder::default(), reducer), transport)
}

fn uploaded(sent: u64) -> Option<Upload> {
    Some(Upload { sent })
}

fn progress(sent: u64) -> FolderAction {
    FolderAction::Upload(PresentationAction::Presented(UploadAction::Progress(sent)))
}

// Detaching.

#[test]
fn with_option_detaching_leaves_the_stream_running() {
    let (mut store, transport) = plain();
    store.send(PlainAction::Attach, |s| s.upload = uploaded(0));
    assert!(transport.frame(0, 10));
    store.receive(PlainAction::Upload(UploadAction::Progress(10)), |s| {
        s.upload = uploaded(10);
    });
    store.send(PlainAction::Detach, |s| s.upload = None);
    // Still open, and still sending progress for an upload that's gone.
    assert_eq!(transport.open_streams(), 1);
    assert!(transport.frame(0, 20));
    store.receive(PlainAction::Upload(UploadAction::Progress(20)), |_| {});
    store.skip_in_flight();
}

#[test]
fn with_a_presentation_dismissing_stops_the_stream() {
    let (mut store, transport) = presented();
    store.send(FolderAction::Attach, |s| s.upload.present(Upload::default()));
    assert!(transport.frame(0, 10));
    store.receive(progress(10), |s| s.upload = uploaded(10).into());
    store.send(FolderAction::Upload(PresentationAction::Dismiss), |s| {
        s.upload = PresentationState::none();
    });
    assert_eq!(transport.open_streams(), 0);
    assert!(!transport.frame(0, 20));
    store.finish();
}

// A late frame from the upload before.

#[test]
fn with_option_a_late_frame_from_upload_one_lands_on_upload_two() {
    let (mut store, transport) = plain();
    store.send(PlainAction::Attach, |s| s.upload = uploaded(0));
    store.send(PlainAction::Detach, |s| s.upload = None);
    store.send(PlainAction::Attach, |s| s.upload = uploaded(0));
    // Upload 1's stream is still open; its frame is taken as upload 2's.
    assert!(transport.frame(0, 70));
    store.receive(PlainAction::Upload(UploadAction::Progress(70)), |s| {
        s.upload = uploaded(70);
    });
    assert_eq!(transport.open_streams(), 2);
    store.skip_in_flight();
}

#[test]
fn with_a_presentation_upload_ones_frames_never_reach_upload_two() {
    let (mut store, transport) = presented();
    store.send(FolderAction::Attach, |s| s.upload.present(Upload::default()));
    store.send(FolderAction::Upload(PresentationAction::Dismiss), |s| {
        s.upload = PresentationState::none();
    });
    store.send(FolderAction::Attach, |s| s.upload.present(Upload::default()));
    assert!(!transport.frame(0, 70));
    // Upload 2's own frames still arrive.
    assert!(transport.frame(1, 5));
    store.receive(progress(5), |s| s.upload = uploaded(5).into());
    assert_eq!(transport.open_streams(), 1);
    store.send(FolderAction::Upload(PresentationAction::Dismiss), |s| {
        s.upload = PresentationState::none();
    });
    store.finish();
}

#[test]
fn with_a_presentation_attaching_over_an_upload_ends_it_first() {
    let (mut store, transport) = presented();
    store.send(FolderAction::Attach, |s| s.upload.present(Upload::default()));
    store.send(FolderAction::Attach, |s| s.upload.present(Upload::default()));
    assert_eq!(transport.open_streams(), 1);
    assert!(!transport.frame(0, 70));
    store.send(FolderAction::Upload(PresentationAction::Dismiss), |s| {
        s.upload = PresentationState::none();
    });
    store.finish();
}

// Ending itself.

#[test]
fn with_option_a_finished_upload_cannot_end_itself() {
    let (mut store, transport) = plain();
    store.send(PlainAction::Attach, |s| s.upload = uploaded(0));
    transport.close(0);
    store.receive(PlainAction::Upload(UploadAction::Finished), |s| s.finished = 1);
    // Its `Effect::dismiss` went nowhere: the parent must drop it by hand.
    assert!(store.state().upload.is_some());
    store.finish();
}

#[test]
fn with_a_presentation_a_finished_upload_ends_itself() {
    let (mut store, transport) = presented();
    store.send(FolderAction::Attach, |s| s.upload.present(Upload::default()));
    transport.close(0);
    store.receive(
        FolderAction::Upload(PresentationAction::Presented(UploadAction::Finished)),
        |s| s.finished = 1,
    );
    store.receive(FolderAction::Upload(PresentationAction::Dismiss), |s| {
        s.upload = PresentationState::none();
    });
    assert_eq!(transport.open_streams(), 0);
    store.finish();
}
