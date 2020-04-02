mod bootstrap;
mod broadcast;
mod rpc;

pub use bootstrap::BootstrapService;
pub use broadcast::BroadcastService;
pub use rpc::RpcService;

use crate::peer_store::PeerStore;

use anyhow::Error;
use lazy_static::lazy_static;
use muta_protocol::{
    traits::{Context, MessageCodec, MessageHandler},
    types::Address,
};
use wormhole::host::QuicHost;

use std::sync::atomic::{AtomicU64, Ordering};

lazy_static! {
    pub static ref NEXT_PROTOCOL_ID: AtomicU64 = AtomicU64::new(1);
}

fn next_protocol_id() -> u64 {
    NEXT_PROTOCOL_ID.fetch_add(1, Ordering::SeqCst)
}

#[derive(Clone)]
pub struct MultiCast(BroadcastService<QuicHost>);

impl MultiCast {
    pub fn new(host: QuicHost, peer_store: PeerStore) -> Self {
        MultiCast(BroadcastService::new(host, peer_store))
    }
}

impl MultiCast {
    pub async fn register_endpoint(
        &self,
        endpoint: &'static str,
        handler: impl MessageHandler,
    ) -> Result<(), Error> {
        self.0.register_then_spawn(endpoint, handler).await
    }

    pub async fn broadcast<M: MessageCodec>(
        &self,
        ctx: Context,
        endpoint: &str,
        msg: M,
    ) -> Result<(), Error> {
        self.0.broadcast(ctx, endpoint, msg).await
    }

    pub async fn multicast_by_chain_addr<M: MessageCodec>(
        &self,
        ctx: Context,
        endpoint: &str,
        chain_addrs: Vec<Address>,
        msg: M,
    ) -> Result<(), Error> {
        self.0.usercast(ctx, endpoint, chain_addrs, msg).await
    }
}

#[derive(Clone)]
pub struct Rpc(RpcService<QuicHost>);

impl Rpc {
    pub fn new(host: QuicHost) -> Self {
        Rpc(RpcService::new(host))
    }
}

impl Rpc {
    pub async fn register_endpoint(
        &self,
        endpoint: &'static str,
        handler: impl MessageHandler,
    ) -> Result<(), Error> {
        self.0.register_then_spawn(endpoint, handler).await
    }

    pub async fn call<P: MessageCodec, R: MessageCodec>(
        &self,
        ctx: Context,
        endpoint: &str,
        mut payload: P,
    ) -> Result<R, Error> {
        todo!()
    }

    pub async fn response<R: MessageCodec>(
        &self,
        ctx: Context,
        result: Result<R, Error>,
    ) -> Result<(), Error> {
        todo!()
    }
}
