use std::{net::SocketAddr, future::Future, task::{Poll, Context as TaskContext}, pin::Pin};

use async_trait::async_trait;
use futures::{
    compat::{Compat01As03, Stream01CompatExt},
    stream::Stream,
    pin_mut
};
use protocol::{
    traits::{Gossip, Context, Priority, MessageCodec},
    ProtocolResult,
    types::UserAddress,
};
use tentacle::{
    builder::ServiceBuilder,
    multiaddr::{Multiaddr, Protocol},
    secio::SecioKeyPair,
    service::{DialProtocol, ProtocolMeta, Service, ServiceControl, TargetSession},
    ProtocolId,
};

use crate::tentacle::error::ServiceError;

pub struct MeasureGossip {
    proto_id: ProtocolId,
    inner:    ServiceControl,
}

#[async_trait]
impl Gossip for MeasureGossip {
    async fn broadcast<M>(&self, _: Context, _: &str, mut msg: M, _: Priority) -> ProtocolResult<()>
    where
        M: MessageCodec,
    {
        let msg_bytes = msg.encode().await?;

        self.inner
            .quick_filter_broadcast(TargetSession::All, self.proto_id, msg_bytes)
            .map_err(ServiceError::from)?;

        Ok(())
    }

    async fn users_cast<M>(&self, _: Context, _: &str, _: Vec<UserAddress>, _: M, _: Priority) -> ProtocolResult<()>
    where
        M: MessageCodec,
    {
        unreachable!()
    }
}

pub struct MeasureService {
    proto_id: ProtocolId,
    inner:    Compat01As03<Service<()>>,
}

impl MeasureService {
    pub fn new(proto_meta: ProtocolMeta) -> Self {
        let proto_id = proto_meta.id();
        let service = ServiceBuilder::default()
            .insert_protocol(proto_meta)
            .key_pair(SecioKeyPair::secp256k1_generated())
            .build(());

        MeasureService {
            proto_id,
            inner: service.compat(),
        }
    }

    pub fn gossip(&mut self) -> MeasureGossip {
        MeasureGossip {
            proto_id: self.proto_id,
            inner:    self.inner.get_mut().control().clone(),
        }
    }

    pub fn listen(&mut self, socket_addr: SocketAddr) -> Result<(), ServiceError> {
        let addr = socket_to_multi_addr(socket_addr);

        self.inner.get_mut().listen(addr)?;

        Ok(())
    }

    pub fn dial(&mut self, peers: Vec<SocketAddr>) -> Result<(), ServiceError> {
        let inner_service = self.inner.get_mut();

        for peer in peers.into_iter() {
            let peer_addr = socket_to_multi_addr(peer);

            inner_service.dial(peer_addr, DialProtocol::All)?;
        }

        Ok(())
    }
}

impl Future for MeasureService {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, ctx: &mut TaskContext) -> Poll<Self::Output> {
        macro_rules! break_ready {
            ($poll:expr) => (
                match $poll {
                    Poll::Pending => break,
                    Poll::Ready(Some(v)) => v,
                    Poll::Ready(None) => return Poll::Ready(()),
                }
            )
        }

        loop {
            let service = &mut self.as_mut().inner;
            pin_mut!(service);

            let _ = break_ready!(service.poll_next(ctx));
        }

        Poll::Pending
    }
}

fn socket_to_multi_addr(socket_addr: SocketAddr) -> Multiaddr {
    let mut multi_addr = Multiaddr::from(socket_addr.ip());
    multi_addr.push(Protocol::Tcp(socket_addr.port()));

    multi_addr
}
