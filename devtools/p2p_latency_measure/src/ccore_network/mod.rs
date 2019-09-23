use std::{
    future::Future,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};

use core_network::{NetworkConfig, NetworkService, NetworkServiceHandle};
use futures::pin_mut;
use protocol::{
    traits::{MessageCodec, MessageHandler},
    ProtocolResult,
};

use crate::config::Config;

pub struct CoreNetworkNode {
    pub service: NetworkService,
}

impl CoreNetworkNode {
    pub fn build(config: &Config) -> Self {
        let config = config.core_network.as_ref().expect("core network config");
        let node_config = NetworkConfig::new();

        let node_config = match config.node.as_str() {
            "bootstrap" => node_config
                .secio_keypair(config.seckey.to_owned())
                .expect("seckey"),
            "peer" => node_config
                .bootstraps(vec![(config.pubkey.to_owned(), config.bootstrap)])
                .expect("bootstrap"),
            _ => panic!("unknown node"),
        };

        CoreNetworkNode {
            service: NetworkService::new(node_config),
        }
    }

    pub fn handle(&self) -> NetworkServiceHandle {
        self.service.handle()
    }

    pub fn listen(&mut self, socket_addr: SocketAddr) {
        self.service.listen(socket_addr).expect("listen");
    }

    pub fn register<M>(
        &mut self,
        endpoint: &str,
        handler: impl MessageHandler<Message = M>,
    ) -> ProtocolResult<()>
    where
        M: MessageCodec + Unpin,
    {
        self.service
            .register_endpoint_handler(endpoint, Box::new(handler))
    }
}

impl Future for CoreNetworkNode {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {
        let service = &mut self.as_mut().service;
        pin_mut!(service);

        service.poll(ctx)
    }
}
