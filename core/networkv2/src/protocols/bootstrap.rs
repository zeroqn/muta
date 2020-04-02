use super::next_protocol_id;
use crate::peer_store::PeerStore;

use anyhow::Error;
use futures::{channel::mpsc, TryFutureExt};
use log::error;
use muta_protocol::traits::Context;
use wormhole::{
    bootstrap::{BootstrapProtocol, Event, Mode},
    crypto::PeerId,
    host::{Host, ProtocolHandler},
    network::ProtocolId,
};

#[derive(Clone)]
pub struct BootstrapService<H: Host + 'static> {
    host:       H,
    proto:      BootstrapProtocol,
    proto_id:   ProtocolId,
    boot_peers: Vec<PeerId>,
}

impl<H: Host + Clone + 'static> BootstrapService<H> {
    pub fn new(
        host: H,
        peer_store: PeerStore,
        mode: Mode,
        new_arrived_tx: mpsc::Sender<Event>,
        boot_peers: Vec<PeerId>,
    ) -> Self {
        let proto_id = next_protocol_id().into();
        println!("proto_id {}", proto_id);

        let proto = BootstrapProtocol::new(proto_id, mode, peer_store, new_arrived_tx);

        BootstrapService {
            host,
            proto,
            proto_id,
            boot_peers,
        }
    }

    pub async fn register_then_spawn(&self) -> Result<(), Error> {
        let proto = self.proto.clone();
        self.host.add_handler(Box::new(proto)).await?;

        for peer in self.boot_peers.clone().into_iter() {
            let host = self.host.clone();
            let proto = self.proto.clone();
            let proto_id = self.proto_id;
            let peer_id = peer.clone();

            let do_boot = async move {
                let stream = host
                    .new_stream(
                        Context::new(),
                        &peer_id,
                        BootstrapProtocol::protocol(proto_id),
                    )
                    .await?;
                proto.handle(stream).await;

                Ok::<(), Error>(())
            };

            tokio::spawn(async move {
                do_boot
                    .unwrap_or_else(|err| {
                        error!("boot peer {} {}", peer, err);
                    })
                    .await;
            });
        }

        Ok::<(), Error>(())
    }
}
