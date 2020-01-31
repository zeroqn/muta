// mod bootstrap;
mod broadcast;
mod discovery;
mod rpc;

// pub use bootstrap::BootstrapService;
pub use broadcast::BroadcastService;
pub use discovery::Discovery;
pub use rpc::RpcService;

use anyhow::Error;
use lazy_static::lazy_static;
use muta_protocol::{
    traits::{Context, MessageCodec, MessageHandler, Priority},
    types::Address,
    ProtocolResult,
};

use std::sync::atomic::{AtomicU64, Ordering};

lazy_static! {
    pub static ref NEXT_PROTOCOL_ID: AtomicU64 = AtomicU64::new(1);
}

fn next_protocol_id() -> u64 {
    NEXT_PROTOCOL_ID.fetch_add(1, Ordering::SeqCst)
}

#[derive(Clone)]
pub struct MultiCast {}

impl MultiCast {
    pub async fn register_endpoint(
        &self,
        endpoint: &'static str,
        handler: impl MessageHandler,
    ) -> Result<(), Error> {
        todo!()
    }

    pub async fn broadcast<M: MessageCodec>(
        &self,
        ctx: Context,
        endpoint: &str,
        msg: M,
    ) -> Result<(), Error> {
        todo!()
    }

    pub async fn multicast_by_chain_addrs<M: MessageCodec>(
        &self,
        ctx: Context,
        endpoint: &str,
        chain_addrs: Vec<Address>,
        msg: M,
    ) -> Result<(), Error> {
        todo!()
    }
}

#[derive(Clone)]
pub struct Rpc {}

impl Rpc {
    pub async fn register_endpoint(
        &self,
        endpoint: &'static str,
        handler: impl MessageHandler,
    ) -> Result<(), Error> {
        todo!()
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
