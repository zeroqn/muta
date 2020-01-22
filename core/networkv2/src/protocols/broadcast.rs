use super::next_protocol_id;
use crate::{context::NetworkContext, peer_store::PeerStore};

use anyhow::{Context as ErrorContext, Error};
use async_trait::async_trait;
use derive_more::Display;
use futures::{self, channel::mpsc, lock::Mutex, SinkExt, TryFutureExt, TryStreamExt};
use log::{debug, error, info, warn};
use muta_protocol::{
    traits::{Context, MessageCodec, MessageHandler},
    types::Address,
};
use wormhole::{
    crypto::PeerId,
    host::{FramedStream, Host, ProtocolHandler},
    network::{Protocol, ProtocolId},
};

use std::{
    borrow::Borrow,
    collections::HashSet,
    future::Future,
    hash::{Hash, Hasher},
    pin::Pin,
    sync::Arc,
    task::{Context as TaskContext, Poll},
};

#[derive(thiserror::Error, Debug)]
pub enum BroadcastError {
    #[error("unregistered endpoint {0}")]
    UnregisteredEndpoint(&'static str),

    #[error("stream closed")]
    StreamClosed,

    #[error("user addresses not found {0:?}")]
    UserAddrsNotFound(Vec<Address>),
}

#[derive(Clone, Display)]
#[display(fmt = "broadcast protocol {}:{}", id, name)]
struct BroadcastProtocol {
    id:   ProtocolId,
    name: &'static str,

    in_stream_tx: Arc<Mutex<mpsc::Sender<FramedStream>>>,
}

impl BroadcastProtocol {
    pub fn new(id: u64, endpoint: &'static str, in_stream_tx: mpsc::Sender<FramedStream>) -> Self {
        BroadcastProtocol {
            id:           id.into(),
            name:         endpoint,
            in_stream_tx: Arc::new(Mutex::new(in_stream_tx)),
        }
    }
}

#[async_trait]
impl ProtocolHandler for BroadcastProtocol {
    fn proto_id(&self) -> ProtocolId {
        self.id
    }

    fn proto_name(&self) -> &'static str {
        self.name
    }

    async fn handle(&self, stream: FramedStream) {
        let mut in_stream_tx = self.in_stream_tx.lock().await;

        if let Err(err) = in_stream_tx.try_send(stream) {
            warn!("broadcast channel {}", err);
        }
    }
}

struct BroadcastReactor<H: MessageHandler<Message = M>, M: MessageCodec> {
    proto:       Protocol,
    stream_rx:   mpsc::Receiver<FramedStream>,
    msg_handler: Arc<H>,
}

impl<H, M> BroadcastReactor<H, M>
where
    H: MessageHandler<Message = M>,
    M: MessageCodec,
{
    pub fn new(proto: Protocol, stream_rx: mpsc::Receiver<FramedStream>, msg_handler: H) -> Self {
        BroadcastReactor {
            proto,
            stream_rx,
            msg_handler: Arc::new(msg_handler),
        }
    }

    pub fn accept(&self, mut stream: FramedStream) -> impl Future<Output = ()> {
        let msg_handler = Arc::clone(&self.msg_handler);
        let remote_peer = stream.conn().remote_peer();
        let proto = self.proto.clone();

        let react = async move {
            let encoded_msg = match stream
                .try_next()
                .await
                .context("read broadcast message")?
                .ok_or(BroadcastError::StreamClosed.into())
            {
                Ok(encoded_msg) => encoded_msg,
                Err(err) => {
                    stream.reset().await;
                    return Err(err);
                }
            };

            let msg = M::decode(encoded_msg)
                .await
                .context("decode broadcast message")?;
            let ctx = Context::new().set_remote_peer(stream.conn().remote_peer());

            if let Err(err) = msg_handler.process(ctx, msg).await {
                error!("process {} {}", proto, err);
            }

            Ok::<(), Error>(())
        };

        react.unwrap_or_else(move |err| warn!("{} broadcast proto {} {}", remote_peer, proto, err))
    }
}

impl<H, M> Future for BroadcastReactor<H, M>
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

struct BroadcastProtocolEndpoint {
    proto:    Protocol,
    endpoint: &'static str,
}

impl BroadcastProtocolEndpoint {
    pub fn new(id: u64, endpoint: &'static str) -> Self {
        BroadcastProtocolEndpoint {
            proto: Protocol::new(id, endpoint),
            endpoint,
        }
    }
}

impl Borrow<str> for BroadcastProtocolEndpoint {
    fn borrow(&self) -> &str {
        &self.endpoint
    }
}

impl PartialEq for BroadcastProtocolEndpoint {
    fn eq(&self, other: &BroadcastProtocolEndpoint) -> bool {
        self.endpoint == other.endpoint
    }
}

impl Eq for BroadcastProtocolEndpoint {}

impl Hash for BroadcastProtocolEndpoint {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        self.endpoint.hash(hasher)
    }
}

#[derive(Clone)]
pub struct BroadcastService<H: Host + Clone + 'static> {
    host:       H,
    peer_store: PeerStore,

    endpoint_book: Arc<Mutex<HashSet<BroadcastProtocolEndpoint>>>,
}

impl<H: Host + Clone + 'static> BroadcastService<H> {
    pub fn new(host: H, peer_store: PeerStore) -> Self {
        BroadcastService {
            host,
            peer_store,

            endpoint_book: Default::default(),
        }
    }

    pub async fn register_then_spawn(
        &self,
        endpoint: &'static str,
        msg_handler: impl MessageHandler,
    ) -> Result<(), Error> {
        debug!("register broadcast protocol for {}", endpoint);

        let proto_id = next_protocol_id();
        let proto = Protocol::new(proto_id, endpoint);
        let (stream_tx, stream_rx) = mpsc::channel(100);

        let generated_proto = BroadcastProtocol::new(proto_id, endpoint, stream_tx);
        self.host.add_handler(Box::new(generated_proto)).await?;

        {
            self.endpoint_book
                .lock()
                .await
                .insert(BroadcastProtocolEndpoint::new(proto_id, endpoint));
        }

        tokio::spawn(BroadcastReactor::new(proto, stream_rx, msg_handler));

        Ok(())
    }

    pub async fn broadcast<M: MessageCodec>(
        &self,
        ctx: Context,
        endpoint: &'static str,
        msg: M,
    ) -> Result<(), Error> {
        let peers = self.host.network().peers().await;
        self.multicast(ctx, endpoint, peers, msg).await?;

        Ok(())
    }

    pub async fn usercast<M: MessageCodec>(
        &self,
        ctx: Context,
        endpoint: &'static str,
        user_addrs: Vec<Address>,
        msg: M,
    ) -> Result<(), Error> {
        let (peers, not_found) = self.peer_store.peers_by_user_addrs(user_addrs).await;
        println!("{:?} {:?}", peers, not_found);

        self.multicast(ctx, endpoint, peers, msg).await?;

        if !not_found.is_empty() {
            Err(BroadcastError::UserAddrsNotFound(not_found).into())
        } else {
            Ok(())
        }
    }

    async fn multicast<M: MessageCodec>(
        &self,
        ctx: Context,
        endpoint: &'static str,
        peers: Vec<PeerId>,
        mut msg: M,
    ) -> Result<(), Error> {
        let encoded_msg = M::encode(&mut msg)
            .await
            .with_context(|| format!("encode broadcast {} message", endpoint))?;

        let proto = {
            self.endpoint_book
                .lock()
                .await
                .get(endpoint)
                .ok_or(BroadcastError::UnregisteredEndpoint(endpoint))?
                .proto
        };

        for peer in peers {
            let proto = proto.clone();
            let host = self.host.clone();
            let msg = encoded_msg.clone();
            let remote_peer = peer.clone();
            let ctx = ctx.clone();

            let do_broadcast = async move {
                let mut stream = host.new_stream(ctx, &peer, proto).await?;

                stream.send(msg).await.context("write broadcast msg")?;

                Ok::<(), Error>(())
            };

            tokio::spawn(async move {
                do_broadcast
                    .unwrap_or_else(move |err| {
                        warn!("{} broadcast proto {} {}", remote_peer, proto, err)
                    })
                    .await
            });
        }

        Ok(())
    }
}
