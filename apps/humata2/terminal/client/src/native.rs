use futures_util::{SinkExt, StreamExt};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use terminal_client::{SendError, Transport, TransportEvent};
use tokio::{runtime::Handle, sync::mpsc, task::JoinHandle};
use tokio_tungstenite::tungstenite::Message;
enum Request {
    Send(String),
    Reconnect,
}
pub struct NativeSocket {
    tx: mpsc::Sender<Request>,
    rx: mpsc::Receiver<TransportEvent>,
    worker: JoinHandle<()>,
    pending: Arc<AtomicUsize>,
}
impl Drop for NativeSocket {
    fn drop(&mut self) {
        self.worker.abort();
    }
}
impl NativeSocket {
    pub fn new(url: String, runtime: &Handle) -> Self {
        let (tx, mut commands) = mpsc::channel(256);
        let (events, rx) = mpsc::channel(256);
        let pending = Arc::new(AtomicUsize::new(0));
        let writes = pending.clone();
        let worker=runtime.spawn(async move{
            loop{
                let (socket,_)=match tokio_tungstenite::connect_async(&url).await{
                    Ok(socket)=>socket,
                    Err(error)=>{let _=events.send(TransportEvent::Failed{operation:"connect to the terminal host",reason:error.to_string()}).await;
                        if wait_for_reconnect(&mut commands, &writes).await {continue;} else {break;}}
                };
                let (mut sink,mut source)=socket.split();if events.send(TransportEvent::Opened).await.is_err(){break;}
                let mut reconnect_now=false;
                loop{
                    tokio::select!{
                        command=commands.recv()=>match command{
                            Some(Request::Send(text))=>{let result=sink.send(Message::Text(text.into())).await;writes.fetch_sub(1,Ordering::Release);if let Err(error)=result{let _=events.send(TransportEvent::Failed{operation:"send terminal input",reason:error.to_string()}).await;break;}},
                            Some(Request::Reconnect)=>{reconnect_now=true;let _=sink.close().await;break;},
                            None=>return,
                        },
                        message=source.next()=>match message{
                            Some(Ok(Message::Text(text)))=>{if text.len()>32*1024*1024{let _=events.send(TransportEvent::Failed{operation:"read a terminal update",reason:"the message exceeded the viewer's 32 MiB safety limit".into()}).await;break;}if events.send(TransportEvent::Text(text.to_string())).await.is_err(){return;}},
                            Some(Ok(Message::Close(frame)))=>{let _=events.send(TransportEvent::Closed{reason:frame.map_or("the host closed the connection".into(),|f|f.reason.to_string())}).await;break;},
                            Some(Ok(Message::Ping(bytes)))=>{let _=sink.send(Message::Pong(bytes)).await;},
                            Some(Ok(_))=>{},
                            Some(Err(error))=>{let _=events.send(TransportEvent::Failed{operation:"read terminal output",reason:error.to_string()}).await;break;},
                            None=>{let _=events.send(TransportEvent::Closed{reason:"the connection ended".into()}).await;break;},
                        }
                    }
                }
                if reconnect_now {continue;}
                // Never replay bytes queued around an uncertain disconnect.
                if !wait_for_reconnect(&mut commands, &writes).await {break;}
            }
        });
        Self {
            tx,
            rx,
            worker,
            pending,
        }
    }
}
// A disconnect never turns stale queued bytes into input on a new connection.
async fn wait_for_reconnect(commands: &mut mpsc::Receiver<Request>, pending: &AtomicUsize) -> bool {
    while let Some(command) = commands.recv().await {
        if matches!(command, Request::Reconnect) {
            while let Ok(command) = commands.try_recv() {
                if matches!(command, Request::Send(_)) {
                    pending.fetch_sub(1, Ordering::Release);
                }
            }
            return true;
        }
        if matches!(command, Request::Send(_)) {
            pending.fetch_sub(1, Ordering::Release);
        }
    }
    false
}
impl Transport for NativeSocket {
    fn drain(&mut self) -> Vec<TransportEvent> {
        let mut out = Vec::new();
        for _ in 0..256 {
            match self.rx.try_recv() {
                Ok(event) => out.push(event),
                Err(_) => break,
            }
        }
        out
    }
    fn send(&mut self, text: String) -> Result<(), SendError> {
        self.pending.fetch_add(1, Ordering::Relaxed);
        self.tx.try_send(Request::Send(text)).map_err(|e| {
            self.pending.fetch_sub(1, Ordering::Release);
            match e {
                mpsc::error::TrySendError::Full(_) => SendError::Full,
                mpsc::error::TrySendError::Closed(_) => SendError::Closed,
            }
        })
    }
    fn reconnect(&mut self) {
        let _ = self.tx.try_send(Request::Reconnect);
    }
    fn is_idle(&self) -> bool {
        self.pending.load(Ordering::Acquire) == 0
    }
}
