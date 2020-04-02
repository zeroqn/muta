use crate::{error::NetworkError, peer_store::PeerStore, protocols::BootstrapService};

use futures::channel::mpsc;
use muta_protocol::traits::Context;
use tokio::sync::Mutex;
use wormhole::{bootstrap, crypto::PeerId, host::Host, multiaddr::Multiaddr};

use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context as TaskContext, Poll},
};

#[derive(Clone)]
pub struct P2p<H: Host + Clone + 'static> {
    host:       H,
    peer_store: PeerStore,

    new_peer_event: Option<Arc<Mutex<mpsc::Receiver<bootstrap::Event>>>>,
}

impl<H: Host + Clone + 'static> P2p<H> {
    pub fn new(host: H, peer_store: PeerStore) -> Self {
        P2p {
            host,
            peer_store,

            new_peer_event: None,
        }
    }
}

impl<H: Host + Clone + 'static> P2p<H> {
    pub async fn listen(&mut self, multiaddr: Multiaddr) -> Result<(), NetworkError> {
        self.host
            .network()
            .listen(multiaddr)
            .await
            .map_err(NetworkError::Listen)
    }

    pub async fn conn_count(&self) -> usize {
        self.host.network().conns().await.len()
    }

    /// Bootstrap will try to connect to at least one of given peers.
    pub async fn bootstrap(
        &mut self,
        ctx: Context,
        bootstrap_peers: Vec<PeerId>,
    ) -> Result<(), NetworkError> {
        let host = self.host.clone();
        let peer_store = self.peer_store.clone();
        let (signal_tx, new_arraived) = mpsc::channel(100);
        let mode = if bootstrap_peers.is_empty() {
            bootstrap::Mode::Publisher
        } else {
            bootstrap::Mode::Subscriber
        };

        let boot = BootstrapService::new(host, peer_store, mode, signal_tx, bootstrap_peers);

        self.new_peer_event = Some(Arc::new(Mutex::new(new_arraived)));
        boot.register_then_spawn()
            .await
            .map_err(NetworkError::Service)
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
