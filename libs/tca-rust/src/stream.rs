//! Effects that follow a stream, such as a gRPC server stream, sending an
//! action for each item until it ends, drops or is cancelled.

use std::future::Future;
use std::pin::Pin;

use tokio::sync::mpsc;

use crate::effect::Effect;

/// A stream of items that may fail partway, as a gRPC stream does when its
/// connection drops. Implement it over your client's stream; channels of
/// results implement it already, which suits fakes.
pub trait EventStream: Send + 'static {
    type Item: Send + 'static;
    type Error: Send + 'static;

    /// The next item, an error once the stream has failed, or None once
    /// it has ended.
    #[allow(clippy::type_complexity)]
    fn next(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Option<Result<Self::Item, Self::Error>>> + Send + '_>>;
}

impl<T: Send + 'static, E: Send + 'static> EventStream for mpsc::UnboundedReceiver<Result<T, E>> {
    type Item = T;
    type Error = E;

    fn next(&mut self) -> Pin<Box<dyn Future<Output = Option<Result<T, E>>> + Send + '_>> {
        Box::pin(self.recv())
    }
}

impl<T: Send + 'static, E: Send + 'static> EventStream for mpsc::Receiver<Result<T, E>> {
    type Item = T;
    type Error = E;

    fn next(&mut self) -> Pin<Box<dyn Future<Output = Option<Result<T, E>>> + Send + '_>> {
        Box::pin(self.recv())
    }
}

/// How a stream an effect followed ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamEnd<E> {
    /// It ended of itself.
    Finished,
    /// It couldn't be opened, or failed partway.
    Failed(E),
}

impl<A: Send + 'static> Effect<A> {
    /// Opens a stream with `open` and sends what `item` makes of each item,
    /// then what `end` makes of how it ended, unless it's cancelled first.
    ///
    /// To resume where it left off after it drops, keep the cursor of the
    /// last item reduced, such as its sequence number, in the state, and
    /// open the next stream from it. Make it
    /// [`cancellable`](Effect::cancellable), cancelling in flight, so a
    /// restart replaces the one running and the feature can stop it.
    pub fn stream<S, Open, Item, End>(open: Open, item: Item, end: End) -> Self
    where
        S: EventStream,
        Open: Future<Output = Result<S, S::Error>> + Send + 'static,
        Item: Fn(S::Item) -> A + Send + 'static,
        End: FnOnce(StreamEnd<S::Error>) -> A + Send + 'static,
    {
        Self::run(move |send| async move {
            let mut stream = match open.await {
                Ok(stream) => stream,
                Err(error) => {
                    send.send(end(StreamEnd::Failed(error)));
                    return;
                }
            };
            loop {
                // The reducer keeps up with the stream, or the stream
                // waits: what isn't read yet stays with its source.
                send.ready().await;
                match stream.next().await {
                    Some(Ok(next)) => send.send(item(next)),
                    Some(Err(error)) => {
                        send.send(end(StreamEnd::Failed(error)));
                        return;
                    }
                    None => {
                        send.send(end(StreamEnd::Finished));
                        return;
                    }
                }
            }
        })
    }
}
