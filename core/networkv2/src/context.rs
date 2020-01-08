use anyhow::Error;
use muta_protocol::traits::Context;
use wormhole::{crypto::PeerId, host::FramedStream};

const STREAM_KEY: &str = "network_stream";
const REMOTE_PEER_KEY: &str = "network_remote_peer";

#[derive(thiserror::Error, Debug)]
pub enum ContextError {
    #[error("no stream found in context")]
    NoStreamFound,

    #[error("no remote peer id found in context")]
    NoRemotePeerFound,
}

pub trait NetworkContext {
    fn set_stream(&self, stream: FramedStream) -> Self;
    fn stream(&self) -> Result<FramedStream, Error>;
    fn set_remote_peer(&self, peer_id: PeerId) -> Self;
    fn remote_peer(&self) -> Result<PeerId, Error>;
}

impl NetworkContext for Context {
    #[must_use]
    fn set_stream(&self, stream: FramedStream) -> Self {
        self.with_value::<FramedStream>(STREAM_KEY, stream)
    }

    fn stream(&self) -> Result<FramedStream, Error> {
        let stream = self
            .get::<FramedStream>(STREAM_KEY)
            .ok_or(ContextError::NoStreamFound)?;

        Ok(stream.clone())
    }

    #[must_use]
    fn set_remote_peer(&self, peer_id: PeerId) -> Self {
        self.with_value::<PeerId>(REMOTE_PEER_KEY, peer_id)
    }

    fn remote_peer(&self) -> Result<PeerId, Error> {
        let remote_peer = self
            .get::<PeerId>(REMOTE_PEER_KEY)
            .ok_or(ContextError::NoRemotePeerFound)?;

        Ok(remote_peer.clone())
    }
}
