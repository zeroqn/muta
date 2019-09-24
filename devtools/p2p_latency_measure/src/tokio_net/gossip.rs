use derive_more::Constructor;
use async_trait::async_trait;
use futures::channel::mpsc::UnboundedSender;
use protocol::{
    traits::{Gossip, Context, Priority, MessageCodec},
    ProtocolResult,
    types::UserAddress,
};
use tentacle::bytes::Bytes;

#[derive(Clone, Constructor)]
pub struct TokioGossip {
    streams: Vec<UnboundedSender<Bytes>>,
}

#[async_trait]
impl Gossip for TokioGossip {
    async fn broadcast<M>(&self, _: Context, _: &str, mut msg: M, _: Priority) -> ProtocolResult<()>
    where
        M: MessageCodec,
    {
        let bytes = msg.encode().await?;

        for stream in self.streams.iter() {
            stream.unbounded_send(bytes.clone()).expect("tokio send bytes");
        }

        Ok(())
    }
    
    async fn users_cast<M>(&self, _: Context, _: &str, _: Vec<UserAddress>, _: M, _: Priority) -> ProtocolResult<()>
    where
        M: MessageCodec
    {
        unreachable!("users cast")
    }
}
