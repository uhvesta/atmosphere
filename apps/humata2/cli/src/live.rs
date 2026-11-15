//! The daemon over its Unix socket: tonic with the bytes passed through
//! as they are, so one client serves every method (D15).

use std::path::PathBuf;

use humata2_proto::humata::v1 as pb;
use hyper_util::rt::TokioIo;
use prost::Message;
use prost::bytes::{Buf, BufMut};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::client::Grpc;
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::codegen::http::uri::PathAndQuery;
use tonic::metadata::{AsciiMetadataValue, MetadataValue};
use tonic::transport::{Channel, Endpoint, Uri};
use tonic::{Request, Status, Streaming};

use crate::daemon::{Daemon, DaemonError, Replies, ReportReply, ReportRequest, Rpc};
use crate::principal::Principal;

/// Replies can be large (a conversation, a diff); requests too (a pasted
/// image is up to 32 MiB, GRPC.md §4).
const MESSAGE_MAX_BYTES: usize = 64 << 20;

pub struct LiveDaemon {
    socket: PathBuf,
    channel: Channel,
    token: Option<(&'static str, AsciiMetadataValue)>,
}

impl LiveDaemon {
    pub async fn connect(socket: PathBuf, principal: &Principal) -> Result<Self, DaemonError> {
        let token = match principal.header() {
            Some((header, token)) => {
                let value = MetadataValue::try_from(token).map_err(|_| DaemonError::BadToken {
                    variable: match principal {
                        Principal::Agent { .. } => "HUMATA_SESSION_TOKEN",
                        _ => "HUMATA_TOKEN",
                    },
                })?;
                Some((header, value))
            }
            None => None,
        };
        let path = socket.clone();
        let channel = Endpoint::from_static("http://[::]:50051")
            .connect_with_connector(tower::service_fn(move |_: Uri| {
                let path = path.clone();
                async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(path).await?)) }
            }))
            .await
            .map_err(|source| DaemonError::Connect {
                socket: socket.clone(),
                source,
            })?;
        Ok(Self {
            socket,
            channel,
            token,
        })
    }

    async fn grpc(&self) -> Result<Grpc<Channel>, DaemonError> {
        let mut grpc = Grpc::new(self.channel.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES);
        grpc.ready().await.map_err(|source| DaemonError::Connect {
            socket: self.socket.clone(),
            source,
        })?;
        Ok(grpc)
    }

    fn request<T>(&self, message: T) -> Request<T> {
        let mut request = Request::new(message);
        if let Some((header, value)) = &self.token {
            request.metadata_mut().insert(*header, value.clone());
        }
        request
    }
}

impl Daemon for LiveDaemon {
    async fn unary(&self, rpc: Rpc, request: Vec<u8>) -> Result<Vec<u8>, DaemonError> {
        let mut grpc = self.grpc().await?;
        let response = grpc
            .unary(self.request(request), PathAndQuery::from_static(rpc.path), Raw)
            .await
            .map_err(|status| refused(rpc, &status))?;
        Ok(response.into_inner())
    }

    async fn server_stream(&self, rpc: Rpc, request: Vec<u8>) -> Result<Replies, DaemonError> {
        let mut grpc = self.grpc().await?;
        let response = grpc
            .server_streaming(self.request(request), PathAndQuery::from_static(rpc.path), Raw)
            .await
            .map_err(|status| refused(rpc, &status))?;
        Ok(pump(rpc, response.into_inner()))
    }

    async fn client_stream(
        &self,
        rpc: Rpc,
        requests: mpsc::Receiver<Vec<u8>>,
    ) -> Result<Vec<u8>, DaemonError> {
        let mut grpc = self.grpc().await?;
        let response = grpc
            .client_streaming(
                self.request(ReceiverStream::new(requests)),
                PathAndQuery::from_static(rpc.path),
                Raw,
            )
            .await
            .map_err(|status| refused(rpc, &status))?;
        Ok(response.into_inner())
    }

    async fn bidi_stream(
        &self,
        rpc: Rpc,
        requests: mpsc::Receiver<Vec<u8>>,
    ) -> Result<Replies, DaemonError> {
        let mut grpc = self.grpc().await?;
        let response = grpc
            .streaming(
                self.request(ReceiverStream::new(requests)),
                PathAndQuery::from_static(rpc.path),
                Raw,
            )
            .await
            .map_err(|status| refused(rpc, &status))?;
        Ok(pump(rpc, response.into_inner()))
    }

    async fn report(&self, _request: ReportRequest) -> Result<ReportReply, DaemonError> {
        Err(DaemonError::NoReportService)
    }
}

/// Moves a stream's replies onto a channel, ending it after an error.
fn pump(rpc: Rpc, mut stream: Streaming<Vec<u8>>) -> Replies {
    let (sender, receiver) = mpsc::channel(16);
    tokio::spawn(async move {
        loop {
            let next = match stream.message().await {
                Ok(Some(message)) => Ok(message),
                Ok(None) => return,
                Err(status) => Err(refused(rpc, &status)),
            };
            let failed = next.is_err();
            if sender.send(next).await.is_err() || failed {
                return;
            }
        }
    });
    receiver
}

/// `google.rpc.Status`, the wrapper a Humata error travels in.
#[derive(Clone, PartialEq, prost::Message)]
struct RpcStatus {
    #[prost(int32, tag = "1")]
    code: i32,
    #[prost(string, tag = "2")]
    message: String,
    #[prost(message, repeated, tag = "3")]
    details: Vec<Any>,
}

/// `google.protobuf.Any`.
#[derive(Clone, PartialEq, prost::Message)]
struct Any {
    #[prost(string, tag = "1")]
    type_url: String,
    #[prost(bytes = "vec", tag = "2")]
    value: Vec<u8>,
}

/// The Humata error in a status's details, or the bare status.
fn refused(rpc: Rpc, status: &Status) -> DaemonError {
    let code = status.code() as i32;
    let error = RpcStatus::decode(status.details()).ok().and_then(|wrapper| {
        wrapper
            .details
            .iter()
            .find(|any| any.type_url.ends_with("/humata.v1.Error"))
            .and_then(|any| pb::Error::decode(any.value.as_slice()).ok())
    });
    match error {
        Some(error) => DaemonError::Refused {
            rpc: rpc.path,
            code,
            error: Box::new(error),
        },
        None => DaemonError::Status {
            rpc: rpc.path,
            code,
            message: status.message().to_owned(),
        },
    }
}

/// Encoded messages in and out, unchanged: the commands encode and decode
/// them, so the client needs no generated code.
#[derive(Clone, Copy, Default)]
struct Raw;

impl Codec for Raw {
    type Encode = Vec<u8>;
    type Decode = Vec<u8>;
    type Encoder = Raw;
    type Decoder = Raw;

    fn encoder(&mut self) -> Self::Encoder {
        Raw
    }

    fn decoder(&mut self) -> Self::Decoder {
        Raw
    }
}

impl Encoder for Raw {
    type Item = Vec<u8>;
    type Error = Status;

    fn encode(&mut self, item: Vec<u8>, dst: &mut EncodeBuf<'_>) -> Result<(), Status> {
        dst.put_slice(&item);
        Ok(())
    }
}

impl Decoder for Raw {
    type Item = Vec<u8>;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Vec<u8>>, Status> {
        let mut message = vec![0; src.remaining()];
        src.copy_to_slice(&mut message);
        Ok(Some(message))
    }
}
