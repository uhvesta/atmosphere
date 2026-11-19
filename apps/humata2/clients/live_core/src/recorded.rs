//! Reports every gRPC call the client makes (which method, how long until
//! the answer's headers, how it ended) to an observer. Bodies and metadata
//! values are never looked at.
//!
//! A call is told once its headers arrive. A stream that later fails (a
//! non-ok `grpc-status` in its trailers, or a body that breaks) is told a
//! second time with the same method and start time and the failure's code
//! (D215). A stream that ends well, or that the app drops, adds nothing.

use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use http_body_util::BodyExt as _;
use tonic::Code;
use tonic::codegen::BoxFuture;
use tonic::codegen::http::{HeaderMap, Request, Response};

/// One call, or one stream failing after its headers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    /// The method path without the leading slash, e.g.
    /// `humata.v1.SessionService/List`.
    pub method: String,
    /// When the request was sent.
    pub started: Instant,
    /// How long until the response headers (or the error) arrived; for a
    /// stream's later failure, how long until it failed.
    pub duration: Duration,
    /// "ok", or the gRPC code's snake_case name ("not_found", ...). A second
    /// call with the same method and `started` is the stream failing after
    /// its headers; a broken body is told with its tonic status code.
    pub status: String,
}

/// Told about each call once its headers arrive, and again if its stream
/// fails later.
pub type Observer = Arc<dyn Fn(Call) + Send + Sync>;

/// A service that reports each call to an observer, if there is one.
#[derive(Clone)]
pub struct Recorded<S> {
    inner: S,
    observer: Option<Observer>,
}

impl<S> Recorded<S> {
    pub fn new(inner: S, observer: Option<Observer>) -> Self {
        Self { inner, observer }
    }
}

/// The snake_case name of a gRPC code.
fn code_name(code: Code) -> &'static str {
    match code {
        Code::Ok => "ok",
        Code::Cancelled => "cancelled",
        Code::Unknown => "unknown",
        Code::InvalidArgument => "invalid_argument",
        Code::DeadlineExceeded => "deadline_exceeded",
        Code::NotFound => "not_found",
        Code::AlreadyExists => "already_exists",
        Code::PermissionDenied => "permission_denied",
        Code::ResourceExhausted => "resource_exhausted",
        Code::FailedPrecondition => "failed_precondition",
        Code::Aborted => "aborted",
        Code::OutOfRange => "out_of_range",
        Code::Unimplemented => "unimplemented",
        Code::Internal => "internal",
        Code::Unavailable => "unavailable",
        Code::DataLoss => "data_loss",
        Code::Unauthenticated => "unauthenticated",
    }
}

/// The code a `grpc-status` header or trailer names, if there is one.
fn code_in(headers: &HeaderMap) -> Option<Code> {
    headers
        .get("grpc-status")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<i32>().ok())
        .map(Code::from_i32)
}

/// How a response's headers say the call ended: a `grpc-status` header
/// (trailers-only answer) names the code; no header means it is going well.
fn status_of<R>(response: &Response<R>) -> Code {
    code_in(response.headers()).unwrap_or(Code::Ok)
}

/// A call whose stream is still being read, to tell if it fails.
#[derive(Clone)]
struct Streaming {
    observer: Observer,
    method: String,
    started: Instant,
}

impl Streaming {
    fn failed(&self, code: Code) {
        (self.observer)(Call {
            method: self.method.clone(),
            started: self.started,
            duration: self.started.elapsed(),
            status: code_name(code).to_owned(),
        });
    }

    /// `body`, telling the observer when its trailers carry a failure or
    /// it breaks. Frames pass through unchanged.
    fn watch(self, body: tonic::body::Body) -> tonic::body::Body {
        let on_break = self.clone();
        let body = body
            .map_frame(move |frame| {
                if let Some(code) = frame.trailers_ref().and_then(code_in)
                    && code != Code::Ok
                {
                    self.failed(code);
                }
                frame
            })
            .inspect_err(move |status: &tonic::Status| on_break.failed(status.code()));
        tonic::body::Body::new(body)
    }
}

impl<S, B> tower::Service<Request<B>> for Recorded<S>
where
    S: tower::Service<Request<B>, Response = Response<tonic::body::Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    B: 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = BoxFuture<S::Response, S::Error>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<B>) -> Self::Future {
        // The clone is the one that was not made ready; the ready one is
        // the one that is called.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let Some(observer) = self.observer.clone() else {
            let future = inner.call(request);
            return Box::pin(future);
        };
        let method = request.uri().path().trim_start_matches('/').to_owned();
        let started = Instant::now();
        let future = inner.call(request);
        Box::pin(async move {
            let result = future.await;
            let code = match &result {
                Ok(response) => status_of(response),
                Err(_) => Code::Unavailable,
            };
            observer(Call {
                method: method.clone(),
                started,
                duration: started.elapsed(),
                status: code_name(code).to_owned(),
            });
            if code != Code::Ok {
                // A trailers-only answer or no answer: nothing follows.
                return result;
            }
            let streaming = Streaming {
                observer,
                method,
                started,
            };
            result.map(|response| response.map(|body| streaming.watch(body)))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::sync::Mutex;

    use http_body_util::Full;
    use tonic::codegen::Bytes;
    use tower::{ServiceExt as _, service_fn};

    use super::*;

    fn collecting() -> (Observer, Arc<Mutex<Vec<Call>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let observer: Observer = Arc::new(move |call| {
            if let Ok(mut seen) = sink.lock() {
                seen.push(call);
            }
        });
        (observer, seen)
    }

    fn request(path: &str) -> Request<()> {
        Request::builder()
            .uri(format!("http://localhost{path}"))
            .body(())
            .expect("a valid request")
    }

    fn statuses(seen: &Mutex<Vec<Call>>) -> Vec<String> {
        seen.lock().expect("lock").iter().map(|call| call.status.clone()).collect()
    }

    /// A body that says `text`, then ends with `grpc-status: code` in its
    /// trailers, as a server stream does.
    fn stream_ending_with(text: &'static str, code: &'static str) -> tonic::body::Body {
        let mut trailers = HeaderMap::new();
        trailers.insert("grpc-status", code.parse().expect("a valid header value"));
        let body = Full::new(Bytes::from_static(text.as_bytes()))
            .map_err(|never: Infallible| -> tonic::Status { match never {} })
            .with_trailers(std::future::ready(Some(Ok(trailers))));
        tonic::body::Body::new(body)
    }

    /// A body that says `text`, then breaks.
    fn stream_breaking_after(text: &'static str) -> tonic::body::Body {
        let body = Full::new(Bytes::from_static(text.as_bytes()))
            .map_err(|never: Infallible| -> tonic::Status { match never {} })
            .with_trailers(std::future::ready(Some(Err(tonic::Status::data_loss(
                "the stream was cut",
            )))));
        tonic::body::Body::new(body)
    }

    #[tokio::test]
    async fn an_answered_call_is_ok_and_names_its_method() {
        let (observer, seen) = collecting();
        let inner = service_fn(|_: Request<()>| async {
            Ok::<_, Infallible>(Response::new(stream_ending_with("hello", "0")))
        });
        let response = Recorded::new(inner, Some(observer))
            .oneshot(request("/humata.v1.SessionService/List"))
            .await
            .expect("answered");
        let body = response.into_body().collect().await.expect("the body arrives");
        assert_eq!(body.to_bytes(), Bytes::from_static(b"hello"));
        let seen = seen.lock().expect("lock");
        assert_eq!(seen.len(), 1, "a call that ends well is told once");
        assert_eq!(seen[0].method, "humata.v1.SessionService/List");
        assert_eq!(seen[0].status, "ok");
    }

    #[tokio::test]
    async fn a_grpc_status_header_names_the_code() {
        let (observer, seen) = collecting();
        let inner = service_fn(|_: Request<()>| async {
            Ok::<_, Infallible>(
                Response::builder()
                    .header("grpc-status", "5")
                    .body(tonic::body::Body::empty())
                    .expect("a valid response"),
            )
        });
        let response = Recorded::new(inner, Some(observer))
            .oneshot(request("/humata.v1.SessionService/Get"))
            .await
            .expect("answered");
        let _ = response.into_body().collect().await;
        assert_eq!(statuses(&seen), ["not_found"]);
    }

    #[tokio::test]
    async fn a_transport_error_is_unavailable_and_still_fails() {
        let (observer, seen) = collecting();
        let inner = service_fn(|_: Request<()>| async {
            Err::<Response<tonic::body::Body>, _>("broken pipe")
        });
        let result = Recorded::new(inner, Some(observer))
            .oneshot(request("/humata.v1.DaemonService/GetServerInfo"))
            .await;
        assert_eq!(result.err(), Some("broken pipe"));
        assert_eq!(statuses(&seen), ["unavailable"]);
    }

    #[tokio::test]
    async fn a_stream_that_fails_after_its_headers_is_told_again_with_its_code() {
        let (observer, seen) = collecting();
        let inner = service_fn(|_: Request<()>| async {
            Ok::<_, Infallible>(Response::new(stream_ending_with("event", "14")))
        });
        let response = Recorded::new(inner, Some(observer))
            .oneshot(request("/humata.v1.EventService/Watch"))
            .await
            .expect("answered");
        assert_eq!(statuses(&seen), ["ok"], "the headers came back well");

        let body = response.into_body().collect().await.expect("the body arrives");

        assert_eq!(body.to_bytes(), Bytes::from_static(b"event"), "the stream reads the same");
        let seen = seen.lock().expect("lock");
        assert_eq!(seen.len(), 2, "the failure is told: {seen:?}");
        assert_eq!(seen[1].method, "humata.v1.EventService/Watch");
        assert_eq!(seen[1].status, "unavailable");
        assert_eq!(seen[1].started, seen[0].started, "both are the same call");
        assert!(seen[1].duration >= seen[0].duration);
    }

    #[tokio::test]
    async fn a_stream_whose_body_breaks_is_told_with_the_breaks_code() {
        let (observer, seen) = collecting();
        let inner = service_fn(|_: Request<()>| async {
            Ok::<_, Infallible>(Response::new(stream_breaking_after("event")))
        });
        let response = Recorded::new(inner, Some(observer))
            .oneshot(request("/humata.v1.EventService/Watch"))
            .await
            .expect("answered");

        let read = response.into_body().collect().await;

        assert_eq!(
            read.err().map(|status| status.code()),
            Some(Code::DataLoss),
            "the break still reaches the caller"
        );
        assert_eq!(statuses(&seen), ["ok", "data_loss"]);
    }

    #[tokio::test]
    async fn without_an_observer_the_answer_passes_through() {
        let inner = service_fn(|_: Request<()>| async {
            Ok::<_, Infallible>(
                Response::builder()
                    .header("grpc-status", "5")
                    .body(stream_ending_with("same", "0"))
                    .expect("a valid response"),
            )
        });
        let response = Recorded::new(inner, None)
            .oneshot(request("/humata.v1.SessionService/List"))
            .await
            .expect("answered");
        assert_eq!(response.headers().get("grpc-status").map(|v| v.as_bytes()), Some(&b"5"[..]));
        let body = response.into_body().collect().await.expect("the body arrives");
        assert_eq!(body.to_bytes(), Bytes::from_static(b"same"));
    }
}
