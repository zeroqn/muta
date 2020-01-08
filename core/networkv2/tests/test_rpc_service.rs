mod common;
use common::CommonError;

use anyhow::Error;
use async_trait::async_trait;
use core_networkv2::{NetworkContext, RpcService};
use muta_protocol::{
    traits::{Context, MessageHandler},
    ProtocolError, ProtocolErrorKind, ProtocolResult,
};
use wormhole::{
    crypto::PublicKey,
    host::QuicHost,
    multiaddr::{Multiaddr, MultiaddrExt},
    network::Connectedness,
    peer_store::{PeerInfo, PeerStore},
};

use std::net::ToSocketAddrs;

const ECHO_ENDPOINT: &str = "/muta/rpc/echo/echoed/1";

struct EchoMessageHandler {
    rpc_service: RpcService<QuicHost>,
}

impl EchoMessageHandler {
    pub fn new(rpc_service: RpcService<QuicHost>) -> Self {
        EchoMessageHandler { rpc_service }
    }
}

#[async_trait]
impl MessageHandler for EchoMessageHandler {
    type Message = String;

    async fn process(&self, ctx: Context, msg: Self::Message) -> ProtocolResult<()> {
        self.rpc_service
            .response(ctx, Ok(msg))
            .await
            .map_err(|err| ProtocolError::new(ProtocolErrorKind::Network, err.into()))?;

        Ok(())
    }
}

async fn make_rpc_service<A: ToSocketAddrs>(
    addr: A,
    peer_store: PeerStore,
) -> Result<(RpcService<QuicHost>, PublicKey, Multiaddr), Error> {
    let (sk, pk) = common::random_keypair();

    let mut sock_addr = addr.to_socket_addrs()?;
    let sock_addr = sock_addr.next().ok_or(CommonError::NoSocketAddress)?;
    let multiaddr = Multiaddr::quic_peer(sock_addr, pk.peer_id());

    let peer_info = PeerInfo::with_all(pk.clone(), Connectedness::Connected, multiaddr.clone());
    peer_store.register(peer_info).await;

    let mut host = QuicHost::make(&sk, peer_store.clone())?;
    host.listen(multiaddr.clone()).await?;

    let rpc_serv = RpcService::new(host);
    Ok((rpc_serv, pk, multiaddr))
}

#[tokio::test]
async fn test_rpc_service() -> Result<(), Error> {
    let alice_store = PeerStore::default();
    let bob_store = PeerStore::default();

    let (alice_serv, ..) = make_rpc_service(("127.0.0.1", 2020), alice_store.clone()).await?;
    let (bob_serv, bob_pk, bob_addr) =
        make_rpc_service(("127.0.0.1", 2021), bob_store.clone()).await?;

    let bob_info = PeerInfo::with_addr(bob_pk.peer_id(), bob_addr);
    alice_store.register(bob_info).await;

    {
        let echo_handler = EchoMessageHandler::new(bob_serv.clone());
        bob_serv
            .register_then_spawn(ECHO_ENDPOINT, echo_handler)
            .await?;
    }

    {
        let echo_handler = EchoMessageHandler::new(alice_serv.clone());
        alice_serv
            .register_then_spawn(ECHO_ENDPOINT, echo_handler)
            .await?;
    }

    let ctx = Context::new().set_remote_peer(bob_pk.peer_id());

    let echoed = alice_serv
        .call::<String, String>(ctx, ECHO_ENDPOINT, "hello world".to_owned())
        .await?;
    assert_eq!(&echoed, "hello world");

    Ok(())
}
