//! Reports every gRPC call the client makes (which method, how long until
//! the answer's headers, how it ended) to an observer. Bodies and metadata
//! values are never looked at.

use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tonic::Code;
use tonic::codegen::http::{Request, Response};
use tonic::codegen::BoxFuture;

/// One finished call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    /// The method path without the leading slash, e.g.
    /// `humata.v1.SessionService/List`.
    pub method: String,
    /// When the request was sent.
    pub started: Instant,
    /// How long until the response headers (or the error) arrived.
    pub duration: Duration,
    /// "ok", or the gRPC code's snake_case name ("not_found", ...).
    pub status: String,
}

/// Told about each call once its headers arrive.
pub type Observer = Arc<dyn Fn(Call) + Send + Sync>;

/// A service that reports each call to an observer, if there is one.
#[derive(Clone)]
pub(crate) struct Recorded<S> {
    inner: S,
    observer: Option<Observer>,
}

impl<S> Recorded<S> {
    pub(crate) fn new(inner: S, observer: Option<Observer>) -> Self {
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

/// How a response's headers say the call ended: a `grpc-status` header
/// (trailers-only answer) names the code; no header means it is going well.
fn status_of<R>(response: &Response<R>) -> &'static str {
    response
        .headers()
        .get("grpc-status")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<i32>().ok())
        .map_or("ok", |code| code_name(Code::from_i32(code)))
}

impl<S, B, R> tower::Service<Request<B>> for Recorded<S>
where
    S: tower::Service<Request<B>, Response = Response<R>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    R: 'static,
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
            let status = match &result {
                Ok(response) => status_of(response),
                Err(_) => "unavailable",
            };
            observer(Call {
                method,
                started,
                duration: started.elapsed(),
                status: status.to_owned(),
            });
            result
        })
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::sync::Mutex;

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

    #[tokio::test]
    async fn an_answered_call_is_ok_and_names_its_method() {
        let (observer, seen) = collecting();
        let inner = service_fn(|_: Request<()>| async {
            Ok::<_, Infallible>(Response::new("hello".to_owned()))
        });
        let response = Recorded::new(inner, Some(observer))
            .oneshot(request("/humata.v1.SessionService/List"))
            .await
            .expect("answered");
        assert_eq!(response.body(), "hello");
        let seen = seen.lock().expect("lock");
        assert_eq!(seen.len(), 1);
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
                    .body(())
                    .expect("a valid response"),
            )
        });
        Recorded::new(inner, Some(observer))
            .oneshot(request("/humata.v1.SessionService/Get"))
            .await
            .expect("answered");
        let seen = seen.lock().expect("lock");
        assert_eq!(seen[0].status, "not_found");
    }

    #[tokio::test]
    async fn a_transport_error_is_unavailable_and_still_fails() {
        let (observer, seen) = collecting();
        let inner = service_fn(|_: Request<()>| async { Err::<Response<()>, _>("broken pipe") });
        let result = Recorded::new(inner, Some(observer))
            .oneshot(request("/humata.v1.DaemonService/GetServerInfo"))
            .await;
        assert_eq!(result.err(), Some("broken pipe"));
        let seen = seen.lock().expect("lock");
        assert_eq!(seen[0].status, "unavailable");
    }

    #[tokio::test]
    async fn without_an_observer_the_answer_passes_through() {
        let inner = service_fn(|_: Request<()>| async {
            Ok::<_, Infallible>(
                Response::builder()
                    .header("grpc-status", "5")
                    .body("same".to_owned())
                    .expect("a valid response"),
            )
        });
        let response = Recorded::new(inner, None)
            .oneshot(request("/humata.v1.SessionService/List"))
            .await
            .expect("answered");
        assert_eq!(response.body(), "same");
        assert_eq!(response.headers().get("grpc-status").map(|v| v.as_bytes()), Some(&b"5"[..]));
    }
}
