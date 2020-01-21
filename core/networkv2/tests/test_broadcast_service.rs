mod common;
use common::CommonError;

use anyhow::Error;
use async_trait::async_trait;
use core_networkv2::BroadcastService;
use futures::{channel::mpsc, lock::Mutex, StreamExt};
use muta_protocol::{
    traits::{Context, MessageHandler},
    ProtocolError, ProtocolErrorKind, ProtocolResult,
};
use wormhole::{
    crypto::PublicKey,
    host::{Host, QuicHost},
    multiaddr::{Multiaddr, MultiaddrExt},
    network::Connectedness,
    peer_store::{PeerInfo, PeerStore},
};

use std::{net::ToSocketAddrs, sync::Arc};

const WHITE_HOLE_ENDPOINT: &str = "/muta/broadcast/whitehole/out/1";

struct WhiteHoleMessageHandler {
    out: Arc<Mutex<mpsc::Sender<String>>>,
}

impl WhiteHoleMessageHandler {
    pub fn new(out: mpsc::Sender<String>) -> Self {
        WhiteHoleMessageHandler {
            out: Arc::new(Mutex::new(out)),
        }
    }
}

#[async_trait]
impl MessageHandler for WhiteHoleMessageHandler {
    type Message = String;

    async fn process(&self, _ctx: Context, msg: Self::Message) -> ProtocolResult<()> {
        self.out
            .lock()
            .await
            .try_send(msg)
            .map_err(|err| ProtocolError::new(ProtocolErrorKind::Network, Box::new(err)))?;

        Ok(())
    }
}

async fn make_broadcast_service<A: ToSocketAddrs>(
    addr: A,
    peer_store: PeerStore,
) -> Result<(BroadcastService<QuicHost>, PublicKey, Multiaddr, QuicHost), Error> {
    let (sk, pk) = common::random_keypair();

    let mut sock_addr = addr.to_socket_addrs()?;
    let sock_addr = sock_addr.next().ok_or(CommonError::NoSocketAddress)?;
    let multiaddr = Multiaddr::quic_peer(sock_addr, pk.peer_id());

    let peer_info = PeerInfo::with_all(pk.clone(), Connectedness::Connected, multiaddr.clone());
    peer_store.register(peer_info).await;

    let mut host = QuicHost::make(&sk, peer_store.clone())?;
    host.listen(multiaddr.clone()).await?;

    let cast_serv = BroadcastService::new(host.clone());
    Ok((cast_serv, pk, multiaddr, host))
}

#[tokio::test]
async fn test_broadcast_service() -> Result<(), Error> {
    let alice_store = PeerStore::default();
    let bob_store = PeerStore::default();
    let ciri_store = PeerStore::default();

    let (alice_serv, .., alice_host) =
        make_broadcast_service(("127.0.0.1", 2020), alice_store.clone()).await?;
    let (bob_serv, bob_pk, bob_addr, ..) =
        make_broadcast_service(("127.0.0.1", 2021), bob_store.clone()).await?;
    let (ciri_serv, ciri_pk, ciri_addr, ..) =
        make_broadcast_service(("127.0.0.1", 2022), ciri_store.clone()).await?;

    let bob_info = PeerInfo::with_addr(bob_pk.peer_id(), bob_addr);
    let ciri_info = PeerInfo::with_addr(ciri_pk.peer_id(), ciri_addr);
    alice_store.register(bob_info).await;
    alice_store.register(ciri_info).await;

    let (alice_tx, _alice_rx) = mpsc::channel(10);
    let (bob_tx, mut bob_rx) = mpsc::channel(10);
    let (ciri_tx, mut ciri_rx) = mpsc::channel(10);

    {
        let whitehole_handler = WhiteHoleMessageHandler::new(alice_tx);
        alice_serv
            .register_then_spawn(WHITE_HOLE_ENDPOINT, whitehole_handler)
            .await?;
    }

    {
        let whitehole_handler = WhiteHoleMessageHandler::new(bob_tx);
        bob_serv
            .register_then_spawn(WHITE_HOLE_ENDPOINT, whitehole_handler)
            .await?;
    }

    {
        let whitehole_handler = WhiteHoleMessageHandler::new(ciri_tx);
        ciri_serv
            .register_then_spawn(WHITE_HOLE_ENDPOINT, whitehole_handler)
            .await?;
    }

    alice_host
        .network()
        .dial_peer(Context::new(), &bob_pk.peer_id())
        .await?;
    alice_host
        .network()
        .dial_peer(Context::new(), &ciri_pk.peer_id())
        .await?;

    tokio::spawn(async move {
        alice_serv
            .broadcast(Context::new(), WHITE_HOLE_ENDPOINT, "from alice".to_owned())
            .await?;

        Ok::<(), Error>(())
    });

    let bob_recv = bob_rx.next().await.ok_or(CommonError::NoMessage)?;
    assert_eq!(bob_recv, "from alice");

    let ciri_recv = ciri_rx.next().await.ok_or(CommonError::NoMessage)?;
    assert_eq!(ciri_recv, "from alice");

    Ok(())
}
