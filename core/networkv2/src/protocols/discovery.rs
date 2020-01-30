use anyhow::Error;
use muta_protocol::traits::Context;

use std::{
    future::Future,
    pin::Pin,
    task::{Context as TaskContext, Poll},
};

#[derive(Clone)]
pub struct Discovery {}

impl Discovery {
    pub(crate) fn pull_peers(&self, ctx: Context, number: usize) -> PullPeers {
        todo!()
    }
}

pub(crate) struct PullPeers {}

impl Future for PullPeers {
    type Output = Result<(), Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        todo!()
    }
}
