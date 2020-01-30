use crate::{
    config::NetworkConfig,
    error::NetworkError,
    p2p::P2p,
    peer_store::{PeerInfoBuilder, PeerStore},
    protocols::{Discovery, MultiCast, Rpc},
};

use anyhow::Error;
use async_trait::async_trait;
use futures::{
    future::{BoxFuture, FutureExt, TryFutureExt},
    pin_mut, ready,
};
use log::{debug, error};
use muta_protocol::{
    traits::{Context, MessageCodec, Priority},
    types::Address,
    ProtocolResult,
};
use wormhole::{
    crypto::{PeerId, PublicKey},
    multiaddr::Multiaddr,
};

use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context as TaskContext, Poll},
};

#[derive(Clone)]
pub struct NetworkHandle {
    multicast: MultiCast,
    rpc:       Rpc,
}

#[async_trait]
impl muta_protocol::traits::Gossip for NetworkHandle {
    async fn broadcast<M>(
        &self,
        ctx: Context,
        endpoint: &str,
        msg: M,
        _p: Priority,
    ) -> ProtocolResult<()>
    where
        M: MessageCodec,
    {
        Ok(self
            .multicast
            .broadcast(ctx, endpoint, msg)
            .err_into::<NetworkError>()
            .await?)
    }

    async fn users_cast<M>(
        &self,
        ctx: Context,
        endpoint: &str,
        chain_addrs: Vec<Address>,
        msg: M,
        _p: Priority,
    ) -> ProtocolResult<()>
    where
        M: MessageCodec,
    {
        Ok(self
            .multicast
            .multicast_by_chain_addrs(ctx, endpoint, chain_addrs, msg)
            .err_into::<NetworkError>()
            .await?)
    }
}

#[async_trait]
impl muta_protocol::traits::Rpc for NetworkHandle {
    async fn call<M, R>(
        &self,
        ctx: Context,
        endpoint: &str,
        msg: M,
        _: Priority,
    ) -> ProtocolResult<R>
    where
        M: MessageCodec,
        R: MessageCodec,
    {
        Ok(self
            .rpc
            .call(ctx, endpoint, msg)
            .err_into::<NetworkError>()
            .await?)
    }

    async fn response<M>(&self, ctx: Context, _: &str, msg: M, _: Priority) -> ProtocolResult<()>
    where
        M: MessageCodec,
    {
        Ok(self
            .rpc
            .response(ctx, Ok(msg))
            .err_into::<NetworkError>()
            .await?)
    }
}

struct FutTask<T>(BoxFuture<'static, T>);

impl<T> Future for FutTask<T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, ctx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        Future::poll(Pin::new(&mut self.as_mut().0), ctx)
    }
}

enum State {
    NoListen,
    Listened,
    Idle,
    Maintain(FutTask<Result<(), Error>>),
}

pub struct Network {
    p2p:        P2p,
    peer_store: PeerStore,

    config: Arc<NetworkConfig>,
    state:  State,

    discovery: Discovery,
    multicast: MultiCast,
    rpc:       Rpc,
}

impl Network {
    pub fn new(config: NetworkConfig) -> Self {
        let peer_store = PeerStore::default();
        let p2p = P2p {};
        let config = Arc::new(config);
        let state = State::NoListen;

        let discovery = Discovery {};
        let multicast = MultiCast {};
        let rpc = Rpc {};

        Network {
            p2p,
            peer_store,

            config,
            state,

            discovery,
            multicast,
            rpc,
        }
    }

    pub async fn listen(&mut self, multiaddr: Multiaddr) -> ProtocolResult<()> {
        Ok(self.p2p.listen().await?)
    }

    pub fn handle(&self) -> NetworkHandle {
        NetworkHandle {
            multicast: self.multicast.clone(),
            rpc:       self.rpc.clone(),
        }
    }

    /// Bootstrap will try to connect to at least one bootstrap peer
    pub async fn bootstrap(
        &mut self,
        ctx: Context,
        bootstrap_peers: Vec<(PublicKey, Multiaddr)>,
    ) -> ProtocolResult<()> {
        let (peer_ids, peer_infos) = bootstrap_peers
            .into_iter()
            .map(|(pubkey, multiaddr)| {
                (
                    pubkey.peer_id(),
                    PeerInfoBuilder::new(pubkey)
                        .net_addrs(vec![multiaddr])
                        .protected()
                        .build(),
                )
            })
            .unzip();

        self.peer_store.register_multi_peers(peer_infos).await;
        Ok(self.p2p.bootstrap(Context::new(), peer_ids).await?)
    }

    fn maintain(&mut self) -> FutTask<Result<(), Error>> {
        let p2p = self.p2p.clone();
        let peer_store = self.peer_store.clone();
        let discovery = self.discovery.clone();
        let config = Arc::clone(&self.config);

        let fut = async move {
            let connected_count = p2p.conn_count().await;

            if connected_count >= config.max_connections {
                return Ok::<(), Error>(());
            }

            let gap = config.max_connections - connected_count;
            let condidate_peers = peer_store.connectable_peers(gap).await;

            if condidate_peers.len() < gap {
                tokio::spawn(discovery.pull_peers(Context::new(), 1000));
            }

            tokio::spawn(p2p.dial(Context::new(), condidate_peers));

            Ok(())
        };

        FutTask(fut.boxed())
    }
}

impl Future for Network {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, ctx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        let mut self_mut = self.as_mut();

        match self_mut.state {
            State::NoListen => panic!("no listen"),
            State::Listened => panic!("no bootstrapped, call `bootstrap()` before spawn"),
            State::Idle => self_mut.state = State::Maintain(self_mut.maintain()),
            State::Maintain(ref mut fut_task) => {
                pin_mut!(fut_task);

                if let Err(err) = ready!(fut_task.poll(ctx)) {
                    error!("maintain {}", err);
                }

                self_mut.state = State::Idle;
            }
        }

        Poll::Pending
    }
}
