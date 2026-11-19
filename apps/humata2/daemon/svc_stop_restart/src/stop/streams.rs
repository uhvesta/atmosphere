//! Ends the streams a daemon is serving once it is leaving (D270).
//!
//! A leaving daemon's graceful shutdown waits for every call it is serving
//! to finish. A unary call finishes by itself, but a stream (an Attach, a
//! Hook, an Inbox wait, a host's uplink) lasts as long as its client keeps
//! it open, so a stop or start sat out the whole shutdown grace whenever a
//! terminal was attached. Once the daemon is leaving, a response with
//! nothing ready to send ends with `UNAVAILABLE`, which clients read as
//! "connect again". Anything a call has ready is sent first, so a unary
//! reply and the last messages of a stream still go out.
//!
//! A stream the daemon ends itself on leaving (the event feed) says so
//! with [`EndsByItself`] on its response. It is left to end cleanly,
//! however slowly its own task gets there, and cut only if it is still
//! open [`ENDS_BY`] after the daemon started leaving.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use http_body::{Body, Frame, SizeHint};
use tokio::sync::watch;
use tonic::Status;
use tonic::codegen::Bytes;
use tonic::codegen::http::{self, HeaderMap, HeaderValue};

/// The gRPC status a cut stream ends with: `UNAVAILABLE`.
const UNAVAILABLE: &str = "14";
/// How long a stream marked [`EndsByItself`] has to end once the daemon
/// is leaving, before it is cut too. Within the daemon's shutdown grace.
const ENDS_BY: Duration = Duration::from_secs(2);
/// What a cut stream's client is told. Plain printable ASCII, as
/// `grpc-message` needs no encoding then.
const LEAVING: &str = "humatad2 is stopping or starting itself again, so this stream ended; connect again in a moment (a daemon that was stopped for good needs starting first)";

/// Put in a response's extensions by a call whose stream ends by itself
/// once the daemon is leaving, so that stream is not cut while it does.
#[derive(Clone, Copy)]
pub struct EndsByItself;

/// Says when the daemon starts leaving.
pub struct Leaving(watch::Sender<bool>);

impl Leaving {
    /// A signal, and the layer that ends the streams once it is given.
    pub fn signal() -> (Self, CutStreamsLayer) {
        let (sender, receiver) = watch::channel(false);
        (Self(sender), CutStreamsLayer { leaving: receiver })
    }

    /// The daemon is leaving: every stream with nothing ready ends now.
    pub fn now(&self) {
        self.0.send_replace(true);
    }
}

/// A tower layer wrapping every response body in a [`Cut`].
#[derive(Clone)]
pub struct CutStreamsLayer {
    leaving: watch::Receiver<bool>,
}

impl<S> tower::Layer<S> for CutStreamsLayer {
    type Service = CutStreams<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CutStreams {
            inner,
            leaving: self.leaving.clone(),
        }
    }
}

#[derive(Clone)]
pub struct CutStreams<S> {
    inner: S,
    leaving: watch::Receiver<bool>,
}

type Responding<E> =
    Pin<Box<dyn Future<Output = Result<http::Response<tonic::body::Body>, E>> + Send>>;

impl<S, Request, B> tower::Service<Request> for CutStreams<S>
where
    S: tower::Service<Request, Response = http::Response<B>>,
    S::Future: Send + 'static,
    S::Error: 'static,
    B: Body<Data = Bytes, Error = Status> + Send + 'static,
{
    type Response = http::Response<tonic::body::Body>;
    type Error = S::Error;
    type Future = Responding<S::Error>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let responding = self.inner.call(request);
        let leaving = self.leaving.clone();
        Box::pin(async move {
            let response = responding.await?;
            let ends_by_itself = response.extensions().get::<EndsByItself>().is_some();
            Ok(response
                .map(|body| tonic::body::Body::new(Cut::new(body, leaving, ends_by_itself))))
        })
    }
}

/// A response body that ends with `UNAVAILABLE` trailers once the daemon
/// is leaving and it has nothing ready to send; one that ends by itself
/// only once [`ENDS_BY`] has passed too.
struct Cut<B> {
    body: Pin<Box<B>>,
    leaving: Pin<Box<dyn Future<Output = ()> + Send>>,
    ended: bool,
}

impl<B> Cut<B> {
    fn new(body: B, mut leaving: watch::Receiver<bool>, ends_by_itself: bool) -> Self {
        Self {
            body: Box::pin(body),
            leaving: Box::pin(async move {
                // A sender gone means the daemon is gone too: leaving.
                let _ = leaving.wait_for(|leaving| *leaving).await;
                if ends_by_itself {
                    tokio::time::sleep(ENDS_BY).await;
                }
            }),
            ended: false,
        }
    }
}

impl<B> Body for Cut<B>
where
    B: Body<Data = Bytes, Error = Status>,
{
    type Data = Bytes;
    type Error = Status;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Status>>> {
        let this = self.get_mut();
        if this.ended {
            return Poll::Ready(None);
        }
        if let Poll::Ready(frame) = this.body.as_mut().poll_frame(cx) {
            return Poll::Ready(frame);
        }
        match this.leaving.as_mut().poll(cx) {
            Poll::Ready(()) => {
                this.ended = true;
                Poll::Ready(Some(Ok(Frame::trailers(trailers()))))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.ended || self.body.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        if self.ended {
            SizeHint::with_exact(0)
        } else {
            self.body.size_hint()
        }
    }
}

fn trailers() -> HeaderMap {
    let mut trailers = HeaderMap::new();
    trailers.insert("grpc-status", HeaderValue::from_static(UNAVAILABLE));
    trailers.insert("grpc-message", HeaderValue::from_static(LEAVING));
    trailers
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::future::poll_fn;
    use std::time::Duration;

    use super::*;

    struct Stand {
        frames: VecDeque<Frame<Bytes>>,
        ends: bool,
    }

    impl Body for Stand {
        type Data = Bytes;
        type Error = Status;

        fn poll_frame(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Bytes>, Status>>> {
            let this = self.get_mut();
            match this.frames.pop_front() {
                Some(frame) => Poll::Ready(Some(Ok(frame))),
                None if this.ends => Poll::Ready(None),
                None => Poll::Pending,
            }
        }
    }

    fn stand(data: &[&'static str], ends: bool) -> Stand {
        Stand {
            frames: data
                .iter()
                .map(|data| Frame::data(Bytes::from_static(data.as_bytes())))
                .collect(),
            ends,
        }
    }

    async fn next(body: &mut Pin<Box<Cut<Stand>>>) -> Option<Frame<Bytes>> {
        poll_fn(|cx| body.as_mut().poll_frame(cx))
            .await
            .map(|frame| frame.expect("a stand-in body never fails"))
    }

    fn data(frame: Option<Frame<Bytes>>) -> Bytes {
        frame
            .expect("a frame")
            .into_data()
            .expect("a data frame")
    }

    fn status_of(frame: Option<Frame<Bytes>>) -> Option<Status> {
        let trailers = frame.expect("a frame").into_trailers().expect("trailers");
        Status::from_header_map(&trailers)
    }

    #[tokio::test]
    async fn a_waiting_stream_ends_unavailable_once_the_daemon_is_leaving() {
        let (leaving, layer) = Leaving::signal();
        let mut body = Box::pin(Cut::new(stand(&["one"], false), layer.leaving.clone(), false));
        assert_eq!(data(next(&mut body).await), "one");
        let waiting = tokio::time::timeout(Duration::from_millis(50), next(&mut body)).await;
        assert!(
            waiting.is_err(),
            "a stream with nothing to send waits while the daemon stays"
        );

        leaving.now();
        let status = status_of(next(&mut body).await).expect("the trailers carry a status");
        assert_eq!(status.code(), tonic::Code::Unavailable);
        assert!(next(&mut body).await.is_none());
        assert!(body.is_end_stream());
    }

    #[tokio::test]
    async fn what_a_call_has_ready_still_goes_out_when_the_daemon_is_leaving() {
        let (leaving, layer) = Leaving::signal();
        leaving.now();
        let mut body = Box::pin(Cut::new(
            stand(&["reply", "last"], true),
            layer.leaving.clone(),
            false,
        ));
        assert_eq!(data(next(&mut body).await), "reply");
        assert_eq!(data(next(&mut body).await), "last");
        assert!(
            next(&mut body).await.is_none(),
            "a reply that ends by itself isn't cut"
        );
    }

    struct EndsLater {
        ends: Pin<Box<tokio::time::Sleep>>,
    }

    impl Body for EndsLater {
        type Data = Bytes;
        type Error = Status;

        fn poll_frame(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Bytes>, Status>>> {
            self.get_mut().ends.as_mut().poll(cx).map(|()| None)
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_stream_that_ends_by_itself_ends_cleanly_however_slowly_within_the_limit() {
        let (leaving, layer) = Leaving::signal();
        let slow = EndsLater {
            ends: Box::pin(tokio::time::sleep(ENDS_BY - Duration::from_millis(1))),
        };
        let mut body = Box::pin(Cut::new(slow, layer.leaving.clone(), true));
        leaving.now();
        let frame = poll_fn(|cx| body.as_mut().poll_frame(cx)).await;
        assert!(
            frame.is_none(),
            "the stream was cut instead of ending by itself"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_stream_that_says_it_ends_by_itself_but_stays_open_is_cut_in_the_end() {
        let (leaving, layer) = Leaving::signal();
        let mut body = Box::pin(Cut::new(stand(&[], false), layer.leaving.clone(), true));
        leaving.now();
        let status = status_of(next(&mut body).await).expect("the trailers carry a status");
        assert_eq!(status.code(), tonic::Code::Unavailable);
    }

    #[tokio::test(start_paused = true)]
    async fn an_unmarked_stream_is_cut_without_waiting() {
        let (leaving, layer) = Leaving::signal();
        let mut body = Box::pin(Cut::new(stand(&[], false), layer.leaving.clone(), false));
        leaving.now();
        let started = tokio::time::Instant::now();
        let status = status_of(next(&mut body).await).expect("the trailers carry a status");
        assert_eq!(status.code(), tonic::Code::Unavailable);
        assert!(
            started.elapsed() < ENDS_BY,
            "an attached terminal's stream waited out the limit"
        );
    }

    #[tokio::test]
    async fn a_daemon_gone_without_saying_ends_its_streams_too() {
        let (leaving, layer) = Leaving::signal();
        let mut body = Box::pin(Cut::new(stand(&[], false), layer.leaving.clone(), false));
        drop(leaving);
        let status = status_of(next(&mut body).await).expect("the trailers carry a status");
        assert_eq!(status.code(), tonic::Code::Unavailable);
    }
}
