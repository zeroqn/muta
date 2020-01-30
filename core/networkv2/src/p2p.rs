use crate::error::NetworkError;

use anyhow::Error;
use muta_protocol::traits::Context;
use wormhole::crypto::PeerId;

use std::{
    future::Future,
    pin::Pin,
    task::{Context as TaskContext, Poll},
};

#[derive(Clone)]
pub struct P2p {}

impl P2p {
    pub async fn listen(&self) -> Result<(), NetworkError> {
        todo!()
    }

    pub async fn conn_count(&self) -> usize {
        todo!()
    }

    /// Bootstrap will try to connect to at least one of given peers.
    pub async fn bootstrap(
        &self,
        ctx: Context,
        bootstrap_peers: Vec<PeerId>,
    ) -> Result<(), NetworkError> {
        todo!()
    }

    pub(crate) fn dial(&self, ctx: Context, peers: Vec<PeerId>) -> Dial {
        todo!()
    }
}

pub(crate) struct Dial {}

impl Future for Dial {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        todo!()
    }
}
