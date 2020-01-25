mod common;
use common::CommonError;

use anyhow::Error;
use async_trait::async_trait;
use core_networkv2::{BootstrapService, PeerStore};
use futures::{channel::mpsc, SinkExt, StreamExt, TryStreamExt};
use log::error;
use muta_protocol::{traits::Context, Bytes};
use wormhole::{
    bootstrap::{Event, Mode},
    crypto::PeerId,
    host::{FramedStream, Host, ProtocolHandler, QuicHost},
    multiaddr::{Multiaddr, MultiaddrExt},
    network::{Connectedness, Protocol, ProtocolId},
    peer_store::PeerInfo,
};

use std::net::ToSocketAddrs;

#[derive(Clone)]
pub struct EchoProtocol;

impl EchoProtocol {
    fn proto() -> Protocol {
        Protocol::new(99, "echo")
    }

    async fn echo(stream: &mut FramedStream) -> Result<(), Error> {
        let msg = stream.try_next().await?.expect("impossible");
        stream.send(msg).await?;

        Ok(())
    }
}

#[async_trait]
impl ProtocolHandler for EchoProtocol {
    fn proto_id(&self) -> ProtocolId {
        99.into()
    }

    fn proto_name(&self) -> &'static str {
        "echo"
    }

    async fn handle(&self, mut stream: FramedStream) {
        if let Err(err) = Self::echo(&mut stream).await {
            error!("echo {}", err);
        }
    }
}

async fn make_bootstrap_srv<A: ToSocketAddrs>(
    addr: A,
    peer_store: PeerStore,
    mode: Mode,
    boot: Vec<PeerId>,
) -> Result<
    (
        QuicHost,
        PeerInfo,
        BootstrapService<QuicHost>,
        mpsc::Receiver<Event>,
    ),
    Error,
> {
    let (sk, pk) = common::random_keypair();

    let mut sock_addr = addr.to_socket_addrs()?;
    let sock_addr = sock_addr.next().ok_or(CommonError::NoSocketAddress)?;
    let multiaddr = Multiaddr::quic_peer(sock_addr, pk.peer_id());

    let peer_info = PeerInfo::with_all(pk.clone(), Connectedness::Connected, multiaddr.clone());
    peer_store.register(peer_info.clone()).await?;

    let mut host = QuicHost::make(&sk, peer_store.inner())?;
    host.add_handler(Box::new(EchoProtocol)).await?;
    host.listen(multiaddr.clone()).await?;

    let (tx, rx) = mpsc::channel(10);
    let bt_srv = BootstrapService::new(host.clone(), peer_store, mode, tx, boot);

    let mut peer_info = peer_info.clone();
    peer_info.set_connectedness(Connectedness::CanConnect);

    Ok((host, peer_info, bt_srv, rx))
}

#[tokio::test]
async fn test_bootstrap_service() -> Result<(), Error> {
    let alice_store = PeerStore::default();
    let bob_store = PeerStore::default();
    let ciri_store = PeerStore::default();

    let (_alice_host, alice_info, alice_bt, ..) = make_bootstrap_srv(
        ("127.0.0.1", 2020),
        alice_store.clone(),
        Mode::Publisher,
        vec![],
    )
    .await?;

    let (_bob_host, bob_info, bob_bt, ..) = make_bootstrap_srv(
        ("127.0.0.1", 2021),
        bob_store.clone(),
        Mode::Publisher,
        vec![alice_info.peer_id().to_owned()],
    )
    .await?;

    let (ciri_host, _ciri_info, ciri_bt, mut ciri_rx) = make_bootstrap_srv(
        ("127.0.0.1", 2022),
        ciri_store.clone(),
        Mode::Subscriber,
        vec![alice_info.peer_id().to_owned()],
    )
    .await?;

    bob_store.register(alice_info.clone()).await?;
    ciri_store.register(alice_info.clone()).await?;

    alice_bt.register_then_spawn().await?;
    bob_bt.register_then_spawn().await?;
    ciri_bt.register_then_spawn().await?;

    ciri_rx.next().await.ok_or(CommonError::NoMessage)?;
    let mut echo_stream = ciri_host
        .new_stream(Context::new(), bob_info.peer_id(), EchoProtocol::proto())
        .await?;

    echo_stream.send(Bytes::from("hello world")).await?;
    let echoed = echo_stream.next().await.ok_or(CommonError::NoMessage)??;

    assert_eq!(&echoed, "hello world");

    Ok::<(), Error>(())
}
