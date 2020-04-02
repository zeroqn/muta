use super::next_protocol_id;
use crate::context::NetworkContext;

use anyhow::{Context as ErrorContext, Error};
use async_trait::async_trait;
use derive_more::Display;
use futures::{channel::mpsc, lock::Mutex, SinkExt, TryFutureExt, TryStreamExt};
use log::{debug, error, info, warn};
use muta_protocol::{
    traits::{Context, MessageCodec, MessageHandler},
    BufMut, Bytes, BytesMut,
};
use serde::{Deserialize, Serialize};
use wormhole::{
    host::{FramedStream, Host, ProtocolHandler},
    network::{Protocol, ProtocolId},
};

use std::{
    borrow::Borrow,
    collections::HashSet,
    future::Future,
    hash::{Hash, Hasher},
    ops::{Deref, DerefMut},
    pin::Pin,
    sync::Arc,
    task::{Context as TaskContext, Poll},
};

// pub const PROTOCOL_PREFIX: &str = "/muta/rpc";

pub const RPC_SUCCESS: u8 = 0;
pub const RPC_ERROR: u8 = 1;

pub const RPC_ERR_MSG_BUSY: &str = "service busy";
pub const RPC_ERR_MSG_SHUTDOWN: &str = "service shutdown";

#[derive(thiserror::Error, Debug)]
pub enum RpcError {
    #[error("unregistered endpoint {0}")]
    UnregisteredEndpoint(String),

    #[error("stream closed")]
    StreamClosed,

    #[error("bad response")]
    BadResponse,

    #[error("rpc response {0}")]
    Response(String),
}

#[derive(Debug, Serialize, Deserialize)]
struct ErrorMessage {
    msg: String,
}

impl ErrorMessage {
    pub fn new(msg: String) -> Self {
        ErrorMessage { msg }
    }

    pub fn busy() -> Self {
        ErrorMessage {
            msg: RPC_ERR_MSG_BUSY.to_owned(),
        }
    }

    pub fn shutdown() -> Self {
        ErrorMessage {
            msg: RPC_ERR_MSG_SHUTDOWN.to_owned(),
        }
    }
}

struct RpcStream(FramedStream);

impl RpcStream {
    pub fn new(stream: FramedStream) -> Self {
        RpcStream(stream)
    }

    pub async fn send_request(&mut self, req: Bytes) -> Result<(), Error> {
        Ok(self.send(req).await.context("send rpc request")?)
    }

    pub async fn response<M: MessageCodec>(&mut self, result: u8, mut msg: M) -> Result<(), Error> {
        let mut resp = BytesMut::new();
        resp.put_u8(result);

        let encoded_msg = msg.encode().await.context("encode rpc response")?;
        resp.extend_from_slice(&encoded_msg);

        Ok(self
            .send(resp.freeze())
            .await
            .context("send rpc response")?)
    }
}

impl Deref for RpcStream {
    type Target = FramedStream;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for RpcStream {
    fn deref_mut(&mut self) -> &mut FramedStream {
        &mut self.0
    }
}

#[derive(Clone, Display)]
#[display(fmt = "rpc endpoint {}:{}", id, endpoint)]
struct RpcProtocol {
    id:       ProtocolId,
    endpoint: &'static str,

    // Sender to deliver inbound stream to registered spawned message handler
    in_stream_tx: Arc<Mutex<mpsc::Sender<FramedStream>>>,
}

impl RpcProtocol {
    pub fn new(id: u64, endpoint: &'static str, in_stream_tx: mpsc::Sender<FramedStream>) -> Self {
        RpcProtocol {
            id: id.into(),
            endpoint,
            in_stream_tx: Arc::new(Mutex::new(in_stream_tx)),
        }
    }
}

#[async_trait]
impl ProtocolHandler for RpcProtocol {
    fn proto_id(&self) -> ProtocolId {
        self.id
    }

    fn proto_name(&self) -> &'static str {
        self.endpoint
    }

    async fn handle(&self, stream: FramedStream) {
        let mut err_stream = RpcStream::new(stream.clone());

        let mut in_stream_tx = self.in_stream_tx.lock().await;
        if let Err(err) = in_stream_tx.try_send(stream) {
            if err.is_full() {
                if let Err(err) = err_stream.response(RPC_ERROR, ErrorMessage::busy()).await {
                    error!("fail to report busy of {} {}", self.endpoint, err);
                    return;
                }
            }

            if err.is_disconnected() {
                if let Err(err) = err_stream
                    .response(RPC_ERROR, ErrorMessage::shutdown())
                    .await
                {
                    error!("fail to report shutdown of {} {}", self.endpoint, err);
                    return;
                }
            }
        }
    }
}

// RpcReactor wrap given message handler, spawned in the background.
// It accepts inbound rpc request stream through a channel, decode
// request, then call its message handler to process it.
struct RpcReactor<H: MessageHandler<Message = M>, M: MessageCodec> {
    proto:       Protocol,
    stream_rx:   mpsc::Receiver<FramedStream>,
    req_handler: Arc<H>,
}

impl<H, M> RpcReactor<H, M>
where
    H: MessageHandler<Message = M>,
    M: MessageCodec,
{
    pub fn new(proto: Protocol, stream_rx: mpsc::Receiver<FramedStream>, req_handler: H) -> Self {
        RpcReactor {
            proto,
            stream_rx,
            req_handler: Arc::new(req_handler),
        }
    }

    pub fn accept(&self, mut stream: FramedStream) -> impl Future<Output = ()> {
        let req_handler = Arc::clone(&self.req_handler);
        let remote_peer = stream.conn().remote_peer();
        let proto = self.proto.clone();

        let react = async move {
            let encoded_req = match stream
                .try_next()
                .await
                .context("read rpc request")?
                .ok_or(RpcError::StreamClosed.into())
            {
                Ok(encoded_req) => encoded_req,
                Err(err) => {
                    stream.reset().await;
                    return Err(err);
                }
            };

            let req = M::decode(encoded_req).await.context("decode rpc request")?;
            let ctx = Context::new().set_stream(stream);

            req_handler.process(ctx, req).await;
            Ok::<(), Error>(())
        };

        react.unwrap_or_else(move |err| warn!("{} rpc proto {} {}", remote_peer, proto, err))
    }
}

impl<H, M> Future for RpcReactor<H, M>
where
    H: MessageHandler<Message = M>,
    M: MessageCodec,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, ctx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        let proto = self.proto;
        let reactor = self.get_mut();

        loop {
            let stream = match futures::Stream::poll_next(Pin::new(&mut reactor.stream_rx), ctx) {
                Poll::Ready(Some(stream)) => stream,
                Poll::Ready(None) => {
                    info!("{} handler exit", proto);
                    return Poll::Ready(());
                }
                Poll::Pending => break,
            };

            tokio::spawn(reactor.accept(stream));
        }

        Poll::Pending
    }
}

struct RpcProtocolEndpoint {
    proto:    Protocol,
    endpoint: &'static str,
}

impl RpcProtocolEndpoint {
    pub fn new(id: u64, endpoint: &'static str) -> Self {
        RpcProtocolEndpoint {
            proto: Protocol::new(id, endpoint),
            endpoint,
        }
    }
}

// Borrow endpoint str
impl Borrow<str> for RpcProtocolEndpoint {
    fn borrow(&self) -> &str {
        &self.endpoint
    }
}

impl PartialEq for RpcProtocolEndpoint {
    fn eq(&self, other: &RpcProtocolEndpoint) -> bool {
        self.endpoint == other.endpoint
    }
}

impl Eq for RpcProtocolEndpoint {}

impl Hash for RpcProtocolEndpoint {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        self.endpoint.hash(hasher)
    }
}

#[derive(Clone)]
pub struct RpcService<H: Host + 'static> {
    host:          H,
    endpoint_book: Arc<Mutex<HashSet<RpcProtocolEndpoint>>>,
}

impl<H: Host + 'static> RpcService<H> {
    pub fn new(host: H) -> Self {
        RpcService {
            host,
            endpoint_book: Default::default(),
        }
    }

    // FIXME: check endpoint, should match "/prefix/comp/method_name/schema_version"
    /// register_rpc_response func will automatically generate a ProtocolHandler
    /// for given ```Endpoint``` and ```MessageHandler```. It use ```Endpoint```
    /// for protocol name.
    pub async fn register_then_spawn(
        &self,
        endpoint: &'static str,
        req_handler: impl MessageHandler,
    ) -> Result<(), Error> {
        debug!("register rpc protocol for {}", endpoint);

        let proto_id = next_protocol_id();
        let proto = Protocol::new(proto_id, endpoint);
        let (stream_tx, stream_rx) = mpsc::channel(100);

        let generated_proto = RpcProtocol::new(proto_id, endpoint, stream_tx);
        self.host.add_handler(Box::new(generated_proto)).await?;
        {
            self.endpoint_book
                .lock()
                .await
                .insert(RpcProtocolEndpoint::new(proto_id, endpoint));
        }

        tokio::spawn(RpcReactor::new(proto, stream_rx, req_handler));

        Ok(())
    }

    pub async fn call<P: MessageCodec, R: MessageCodec>(
        &self,
        ctx: Context,
        endpoint: &str,
        mut payload: P,
    ) -> Result<R, Error> {
        let encoded_payload = P::encode(&mut payload)
            .await
            .with_context(|| format!("encode rpc {} request", endpoint))?;
        let remote_peer = ctx
            .remote_peer()
            .with_context(|| format!("rpc {} call", endpoint))?;
        let proto = {
            self.endpoint_book
                .lock()
                .await
                .get(endpoint)
                .ok_or(RpcError::UnregisteredEndpoint(endpoint.to_owned()))?
                .proto
        };

        let rpc_stream = self
            .host
            .new_stream(ctx, &remote_peer, proto)
            .await
            .with_context(|| format!("create rpc {} stream", endpoint))?;
        let mut rpc_stream = RpcStream::new(rpc_stream);

        rpc_stream.send_request(encoded_payload).await?;
        let mut resp = rpc_stream
            .try_next()
            .await
            .with_context(|| format!("wait {} rpc response", endpoint))?
            .ok_or(RpcError::StreamClosed)?;

        if resp.len() < 1 {
            return Err(RpcError::BadResponse).context("empty response");
        }
        let encoded_resp_body = resp.split_off(1);

        if RPC_SUCCESS != resp[0] {
            let err_msg = ErrorMessage::decode(encoded_resp_body)
                .await
                .with_context(|| format!("decode {} rpc error message", endpoint))?;

            Err(RpcError::Response(err_msg.msg).into())
        } else {
            Ok(R::decode(encoded_resp_body)
                .await
                .with_context(|| format!("decode {} rpc response", endpoint))?)
        }
    }

    pub async fn response<R: MessageCodec>(
        &self,
        ctx: Context,
        result: Result<R, Error>,
    ) -> Result<(), Error> {
        let rpc_stream = ctx.stream().with_context(|| format!("rpc response"))?;
        let mut rpc_stream = RpcStream::new(rpc_stream);

        match result {
            Ok(result) => rpc_stream.response(RPC_SUCCESS, result).await,
            Err(err) => {
                rpc_stream
                    .response(RPC_ERROR, ErrorMessage::new(format!("{}", err)))
                    .await
            }
        }
    }
}
