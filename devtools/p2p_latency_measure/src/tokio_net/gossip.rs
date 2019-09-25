use std::net::SocketAddr;

use async_trait::async_trait;
use derive_more::Constructor;
use futures::{
    channel::mpsc::{unbounded, UnboundedSender},
    sink::SinkExt,
};
use log::info;
use protocol::{
    traits::{Context, Gossip, MessageCodec, Priority},
    types::UserAddress,
    ProtocolResult,
};
use tentacle::bytes::Bytes;
use tokio::{
    codec::{Framed, LengthDelimitedCodec},
    net::tcp::TcpStream,
};

#[derive(Clone, Constructor)]
pub struct TokioGossip {
    streams: Vec<UnboundedSender<Bytes>>,
}

impl TokioGossip {
    pub fn from(socket_addrs: Vec<SocketAddr>) -> Self {
        let mut streams = Vec::new();

        for addr in socket_addrs.into_iter() {
            let (tx, mut rx) = unbounded();
            let stream_fut = TcpStream::connect(addr);

            streams.push(tx);

            let gossip_msg = async move {
                let stream = stream_fut.await.expect("tcp stream");
                let mut gossip_framed = Framed::new(stream, LengthDelimitedCodec::new());

                if gossip_framed.send_all(&mut rx).await.is_err() {
                    info!("gossip stream closed");
                }
            };

            runtime::spawn(gossip_msg);
        }

        TokioGossip { streams }
    }
}

#[async_trait]
impl Gossip for TokioGossip {
    async fn broadcast<M>(&self, _: Context, _: &str, mut msg: M, _: Priority) -> ProtocolResult<()>
    where
        M: MessageCodec,
    {
        let bytes = msg.encode().await?;

        for stream in self.streams.iter() {
            stream
                .unbounded_send(bytes.clone())
                .expect("tokio send bytes");
        }

        Ok(())
    }

    async fn users_cast<M>(
        &self,
        _: Context,
        _: &str,
        _: Vec<UserAddress>,
        _: M,
        _: Priority,
    ) -> ProtocolResult<()>
    where
        M: MessageCodec,
    {
        unreachable!("users cast")
    }
}
