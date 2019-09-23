mod error;
mod mprotocol;
mod recorder;
mod service;

use mprotocol::MeasureProtocol;
use recorder::Recorder;
use service::{MeasureGossip, MeasureService};

use std::{
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{
    channel::mpsc::{unbounded, UnboundedReceiver},
    pin_mut,
};
use protocol::{
    traits::{MessageCodec, MessageHandler},
    ProtocolResult,
};
use tentacle::bytes::Bytes;

use crate::config::Config;

const PROTOCOL_ID: usize = 1;

pub struct TentacleNode {
    service: MeasureService,
    msg_rx:  Option<UnboundedReceiver<Bytes>>,
}

impl TentacleNode {
    pub fn build(config: &Config) -> Self {
        let (msg_tx, msg_rx) = unbounded();

        let protocol = MeasureProtocol::new(msg_tx);
        let proto_meta = protocol.build_meta(PROTOCOL_ID);

        let mut service = MeasureService::new(proto_meta);
        let tentacle_config = config.tentacle.as_ref().expect("no tentacle bootstraps");
        let bootstraps = tentacle_config.bootstraps.clone();

        service.dial(bootstraps).expect("dial failure");

        TentacleNode {
            service,
            msg_rx: Some(msg_rx),
        }
    }

    pub fn handle(&mut self) -> MeasureGossip {
        self.service.gossip()
    }

    pub fn listen(&mut self, socket_addr: SocketAddr) {
        self.service.listen(socket_addr).expect("listen");
    }

    pub fn register<M>(
        &mut self,
        _: &str,
        handler: impl MessageHandler<Message = M>,
    ) -> ProtocolResult<()>
    where
        M: MessageCodec + Unpin,
    {
        let msg_rx = self.msg_rx.take().expect("none msg_rx");
        let recorder = Recorder::new(msg_rx, Arc::new(handler));

        runtime::spawn(recorder);

        Ok(())
    }
}

impl Future for TentacleNode {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {
        let service = &mut self.as_mut().service;
        pin_mut!(service);

        service.poll(ctx)
    }
}
