use std::{io::Error, net::SocketAddr, sync::Arc, thread, time::Duration};

use parking_lot::RwLock;
use async_trait::async_trait;
use derive_more::Constructor;
use futures::{
    channel::mpsc::{unbounded, UnboundedSender},
    compat::Future01CompatExt,
    stream::{StreamExt, TryStreamExt},
};
use futures01::sink::Sink;
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

type GossipStreams = Arc<RwLock<Vec<UnboundedSender<Result<Bytes, Error>>>>>;

#[derive(Clone, Constructor)]
pub struct TokioGossip {
    streams: GossipStreams,
}

impl TokioGossip {
    pub fn from(socket_addrs: Vec<SocketAddr>) -> Self {
        let streams = Arc::new(RwLock::new(Vec::new()));

        for addr in socket_addrs.into_iter() {
            let (tx, rx) = unbounded();
            let stream_fut = TcpStream::connect(&addr);
            let streams = Arc::clone(&streams);

            let gossip_msg = async move {
                thread::sleep(Duration::from_secs(3));

                let mut rx = rx.boxed().compat();
                let stream = stream_fut.compat().await.expect("tcp stream");
                let gossip_framed = Framed::new(stream, LengthDelimitedCodec::new());

                streams.write().push(tx);

                if gossip_framed.send_all(&mut rx).compat().await.is_err() {
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
        let streams = self.streams.read();

        for stream in streams.iter() {
            stream
                .unbounded_send(Ok(bytes.clone()))
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
