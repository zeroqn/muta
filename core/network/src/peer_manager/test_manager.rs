use super::{
    addr_info::{ADDR_TIMEOUT, MAX_ADDR_RETRY},
    peer::{Peer, VALID_ATTEMPT_INTERVAL},
    AddrInfo, ArcPeer, ArcProtectedPeer, Connectedness, Inner, PeerManager, PeerManagerConfig,
    PeerMultiaddr, ALIVE_RETRY_INTERVAL, MAX_RETRY_COUNT, WHITELIST_TIMEOUT,
};
use crate::{
    common::ConnectedAddr,
    event::{ConnectionEvent, ConnectionType, PeerManagerEvent, RemoveKind, RetryKind},
    test::mock::SessionContext,
    traits::MultiaddrExt,
};

use futures::{
    channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender},
    StreamExt,
};
use tentacle::{
    multiaddr::Multiaddr,
    secio::{PeerId, PublicKey, SecioKeyPair},
    service::SessionType,
    SessionId,
};

use std::{
    borrow::Cow,
    collections::HashSet,
    convert::TryInto,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

fn make_multiaddr(port: u16, id: Option<PeerId>) -> Multiaddr {
    let mut multiaddr = format!("/ip4/127.0.0.1/tcp/{}", port)
        .parse::<Multiaddr>()
        .expect("peer multiaddr");

    if let Some(id) = id {
        multiaddr.push_id(id);
    }

    multiaddr
}

fn make_peer_multiaddr(port: u16, id: PeerId) -> PeerMultiaddr {
    make_multiaddr(port, Some(id))
        .try_into()
        .expect("try into peer multiaddr")
}

fn make_peer(port: u16) -> ArcPeer {
    let keypair = SecioKeyPair::secp256k1_generated();
    let pubkey = keypair.public_key();
    let peer_id = pubkey.peer_id();
    let peer = ArcPeer::from_pubkey(pubkey).expect("make peer");
    let multiaddr = make_peer_multiaddr(port, peer_id);

    peer.set_multiaddrs(vec![multiaddr]);
    peer
}

fn make_bootstraps(num: usize) -> Vec<ArcPeer> {
    let mut init_port = 5000;

    (0..num)
        .map(|_| {
            let peer = make_peer(init_port);
            init_port += 1;
            peer
        })
        .collect()
}

struct MockManager {
    event_tx: UnboundedSender<PeerManagerEvent>,
    inner:    PeerManager,
}

impl MockManager {
    pub fn new(inner: PeerManager, event_tx: UnboundedSender<PeerManagerEvent>) -> Self {
        MockManager { event_tx, inner }
    }

    pub fn config(&self) -> &PeerManagerConfig {
        &self.inner.config
    }

    pub async fn poll_event(&mut self, event: PeerManagerEvent) {
        self.event_tx.unbounded_send(event).expect("send event");
        self.await
    }

    pub async fn poll(&mut self) {
        self.await
    }

    pub fn unknown_book(&self) -> &HashSet<AddrInfo> {
        &self.inner.unknown_addrs
    }

    pub fn unknown_book_mut(&mut self) -> &mut HashSet<AddrInfo> {
        &mut self.inner.unknown_addrs
    }

    pub fn core_inner(&self) -> Arc<Inner> {
        self.inner.inner()
    }
}

impl Future for MockManager {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Self::Output> {
        let _ = Future::poll(Pin::new(&mut self.as_mut().inner), ctx);
        Poll::Ready(())
    }
}

fn make_manager(
    bootstrap_num: usize,
    max_connections: usize,
) -> (MockManager, UnboundedReceiver<ConnectionEvent>) {
    let manager_pubkey = make_pubkey();
    let manager_id = manager_pubkey.peer_id();
    let bootstraps = make_bootstraps(bootstrap_num);
    let mut peer_dat_file = std::env::temp_dir();
    peer_dat_file.push("peer.dat");

    let config = PeerManagerConfig {
        our_id: manager_id,
        pubkey: manager_pubkey,
        bootstraps,
        max_connections,
        routine_interval: Duration::from_secs(10),
        peer_dat_file,
    };

    let (conn_tx, conn_rx) = unbounded();
    let (mgr_tx, mgr_rx) = unbounded();
    let manager = PeerManager::new(config, mgr_rx, conn_tx);

    (MockManager::new(manager, mgr_tx), conn_rx)
}

fn make_pubkey() -> PublicKey {
    let keypair = SecioKeyPair::secp256k1_generated();
    keypair.public_key()
}

async fn make_sessions(mgr: &mut MockManager, num: usize) -> Vec<ArcPeer> {
    let mut next_sid = 1;
    let mut peers = Vec::with_capacity(num);
    let inner = mgr.core_inner();

    for _ in (0..num).into_iter() {
        let remote_pubkey = make_pubkey();
        let remote_pid = remote_pubkey.peer_id();
        let remote_addr = make_multiaddr(6000, Some(remote_pid.clone()));

        let sess_ctx = SessionContext::make(
            SessionId::new(next_sid),
            remote_addr.clone(),
            SessionType::Outbound,
            remote_pubkey.clone(),
        );
        next_sid += 1;

        let new_session = PeerManagerEvent::NewSession {
            pid:    remote_pid.clone(),
            pubkey: remote_pubkey,
            ctx:    sess_ctx.arced(),
        };
        mgr.poll_event(new_session).await;

        peers.push(inner.peer(&remote_pid).expect("make peer session"));
    }

    assert_eq!(inner.connected(), num, "make some sessions");
    peers
}

#[tokio::test]
async fn should_accept_outbound_new_session_and_add_peer() {
    let (mut mgr, _conn_rx) = make_manager(2, 20);

    let remote_pubkey = make_pubkey();
    let remote_addr = make_multiaddr(6000, Some(remote_pubkey.peer_id()));
    let sess_ctx = SessionContext::make(
        SessionId::new(1),
        remote_addr.clone(),
        SessionType::Outbound,
        remote_pubkey.clone(),
    );

    let new_session = PeerManagerEvent::NewSession {
        pid:    remote_pubkey.peer_id(),
        pubkey: remote_pubkey.clone(),
        ctx:    sess_ctx.arced(),
    };
    mgr.poll_event(new_session).await;

    let inner = mgr.core_inner();
    assert_eq!(inner.connected(), 1, "should have one without bootstrap");

    let saved_peer = inner
        .peer(&remote_pubkey.peer_id())
        .expect("should save peer");
    assert_eq!(saved_peer.session_id(), 1.into());
    assert_eq!(saved_peer.connectedness(), Connectedness::Connected);
    let saved_addrs_len = saved_peer.multiaddrs_len();
    assert_eq!(saved_addrs_len, 1, "should save outbound multiaddr");
    assert!(
        saved_peer.raw_multiaddrs().contains(&remote_addr),
        "should contain remote multiadr"
    );
    assert_eq!(saved_peer.retry(), 0, "should reset retry");

    let saved_session = inner.session(1.into()).expect("should save session");
    assert_eq!(saved_session.peer.id.as_ref(), &remote_pubkey.peer_id());
    assert!(!saved_session.is_blocked());
    assert_eq!(
        saved_session.connected_addr,
        ConnectedAddr::from(&remote_addr)
    );
}

#[tokio::test]
async fn should_ignore_inbound_address_on_new_session() {
    let (mut mgr, _conn_rx) = make_manager(2, 20);

    let remote_pubkey = make_pubkey();
    let remote_addr = make_multiaddr(6000, Some(remote_pubkey.peer_id()));
    let sess_ctx = SessionContext::make(
        SessionId::new(1),
        remote_addr.clone(),
        SessionType::Inbound,
        remote_pubkey.clone(),
    );

    let new_session = PeerManagerEvent::NewSession {
        pid:    remote_pubkey.peer_id(),
        pubkey: remote_pubkey.clone(),
        ctx:    sess_ctx.arced(),
    };
    mgr.poll_event(new_session).await;

    let inner = mgr.core_inner();
    assert_eq!(inner.connected(), 1, "should have one without bootstrap");

    let saved_peer = inner
        .peer(&remote_pubkey.peer_id())
        .expect("should save peer");
    let saved_addrs_len = saved_peer.multiaddrs_len();
    assert_eq!(saved_addrs_len, 0, "should not save inbound multiaddr");
}

#[tokio::test]
async fn should_enforce_id_in_multiaddr_on_new_session() {
    let (mut mgr, _conn_rx) = make_manager(2, 20);

    let remote_pubkey = make_pubkey();
    let remote_addr = make_multiaddr(6000, None);
    let sess_ctx = SessionContext::make(
        SessionId::new(1),
        remote_addr.clone(),
        SessionType::Outbound,
        remote_pubkey.clone(),
    );

    let new_session = PeerManagerEvent::NewSession {
        pid:    remote_pubkey.peer_id(),
        pubkey: remote_pubkey.clone(),
        ctx:    sess_ctx.arced(),
    };
    mgr.poll_event(new_session).await;

    let inner = mgr.core_inner();
    assert_eq!(inner.connected(), 1, "should have one without bootstrap");

    let saved_peer = inner
        .peer(&remote_pubkey.peer_id())
        .expect("should save peer");
    let saved_addrs = saved_peer.raw_multiaddrs();
    assert_eq!(saved_addrs.len(), 1, "should save outbound multiaddr");

    let remote_addr = saved_addrs.first().expect("get first multiaddr");
    assert!(remote_addr.has_id());
    assert_eq!(
        remote_addr.id_bytes(),
        Some(Cow::Borrowed(remote_pubkey.peer_id().as_bytes())),
        "id should match"
    );
}

#[tokio::test]
async fn should_add_multiaddr_to_peer_on_new_session() {
    let (mut mgr, _conn_rx) = make_manager(2, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;

    let inner = mgr.core_inner();
    assert_eq!(inner.connected(), 1, "should have one without bootstrap");

    let test_peer = remote_peers.first().expect("get first");
    let session_closed = PeerManagerEvent::SessionClosed {
        pid: test_peer.owned_id(),
        sid: test_peer.session_id(),
    };
    mgr.poll_event(session_closed).await;

    let new_multiaddr = make_multiaddr(9999, None);
    let sess_ctx = SessionContext::make(
        SessionId::new(2),
        new_multiaddr,
        SessionType::Outbound,
        test_peer.owned_pubkey(),
    );
    let new_session = PeerManagerEvent::NewSession {
        pid:    test_peer.owned_id(),
        pubkey: test_peer.owned_pubkey(),
        ctx:    sess_ctx.arced(),
    };
    mgr.poll_event(new_session).await;

    assert_eq!(test_peer.multiaddrs_len(), 2, "should have 2 addrs");
}

#[tokio::test]
async fn should_dec_connecting_and_inc_connected_for_connecting_peer_on_new_session() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let test_peer = make_peer(2020);

    let inner = mgr.core_inner();
    assert_eq!(inner.connected(), 0, "should not have any connection");
    assert_eq!(inner.connecting(), 0, "should not have any connection");

    inner.add_peer(test_peer.clone());
    mgr.poll().await;
    assert_eq!(inner.connecting(), 1, "should try connect test peer");
    assert_eq!(test_peer.connectedness(), Connectedness::Connecting);

    let sess_ctx = SessionContext::make(
        SessionId::new(1),
        test_peer.raw_multiaddrs().pop().expect("get multiaddr"),
        SessionType::Outbound,
        test_peer.owned_pubkey(),
    );
    let new_session = PeerManagerEvent::NewSession {
        pid:    test_peer.owned_id(),
        pubkey: test_peer.owned_pubkey(),
        ctx:    sess_ctx.arced(),
    };
    mgr.poll_event(new_session).await;

    assert_eq!(test_peer.connectedness(), Connectedness::Connected);
    assert_eq!(inner.connected(), 1, "should have one connected");
    assert_eq!(inner.connecting(), 0, "should not have any connecting");
}

#[tokio::test]
async fn should_always_remove_inbound_mutiaddr_in_unknown_book_even_when_reach_max_connections_on_new_session(
) {
    let (mut mgr, _conn_rx) = make_manager(0, 2);
    let _remote_peers = make_sessions(&mut mgr, 2).await;

    let remote_pubkey = make_pubkey();
    mgr.unknown_book_mut()
        .insert(make_peer_multiaddr(9527, remote_pubkey.peer_id()).into());
    assert_eq!(mgr.unknown_book().len(), 1, "should have one unknown addr");

    let sess_ctx = SessionContext::make(
        SessionId::new(1),
        make_multiaddr(9527, None),
        SessionType::Inbound,
        remote_pubkey.clone(),
    );
    let new_session = PeerManagerEvent::NewSession {
        pid:    remote_pubkey.peer_id(),
        pubkey: remote_pubkey.clone(),
        ctx:    sess_ctx.arced(),
    };
    mgr.poll_event(new_session).await;

    let inner = mgr.core_inner();
    assert_eq!(mgr.unknown_book().len(), 0, "should remove unknown addr");
    assert_eq!(inner.connected(), 2, "should not increase conn count");
}

#[tokio::test]
async fn should_remove_same_mutiaddr_in_unknown_book_on_new_session() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);

    let remote_pubkey = make_pubkey();
    let test_addr = make_multiaddr(2077, Some(remote_pubkey.peer_id()));

    mgr.unknown_book_mut()
        .insert(make_peer_multiaddr(2077, remote_pubkey.peer_id()).into());
    assert_eq!(mgr.unknown_book().len(), 1, "should have one unknown addr");

    let sess_ctx = SessionContext::make(
        SessionId::new(1),
        test_addr,
        SessionType::Outbound,
        remote_pubkey.clone(),
    );

    let new_session = PeerManagerEvent::NewSession {
        pid:    remote_pubkey.peer_id(),
        pubkey: remote_pubkey.clone(),
        ctx:    sess_ctx.arced(),
    };
    mgr.poll_event(new_session).await;

    let inner = mgr.core_inner();
    assert_eq!(inner.connected(), 1, "should have one connection");
    assert_eq!(
        mgr.unknown_book().len(),
        0,
        "should remove same unknown addr"
    );
}

#[tokio::test]
async fn should_remove_same_multiaddr_in_unknown_book_if_ctx_address_doesnt_have_id_on_new_session()
{
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let remote_pubkey = make_pubkey();
    mgr.unknown_book_mut()
        .insert(make_peer_multiaddr(2077, remote_pubkey.peer_id()).into());

    assert_eq!(mgr.unknown_book().len(), 1, "should have one unknown addr");

    let sess_ctx = SessionContext::make(
        SessionId::new(1),
        make_multiaddr(2077, None), // Multiaddr without id in ctx
        SessionType::Outbound,
        remote_pubkey.clone(),
    );

    let new_session = PeerManagerEvent::NewSession {
        pid:    remote_pubkey.peer_id(),
        pubkey: remote_pubkey.clone(),
        ctx:    sess_ctx.arced(),
    };
    mgr.poll_event(new_session).await;

    let inner = mgr.core_inner();
    assert_eq!(inner.connected(), 1, "should have one connection");
    assert_eq!(
        mgr.unknown_book().len(),
        0,
        "should remove same unknown addr"
    );
}

#[tokio::test]
async fn should_reject_new_connection_for_same_peer_on_new_session() {
    let (mut mgr, mut conn_rx) = make_manager(0, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;

    let test_peer = remote_peers.first().expect("get first peer");
    let expect_sid = test_peer.session_id();
    let sess_ctx = SessionContext::make(
        SessionId::new(99),
        test_peer.raw_multiaddrs().pop().expect("get multiaddr"),
        SessionType::Outbound,
        test_peer.owned_pubkey(),
    );

    let new_session = PeerManagerEvent::NewSession {
        pid:    test_peer.owned_id(),
        pubkey: test_peer.owned_pubkey(),
        ctx:    sess_ctx.arced(),
    };
    mgr.poll_event(new_session).await;

    let inner = mgr.core_inner();
    assert_eq!(inner.connected(), 1, "should not increase conn count");
    assert_eq!(
        test_peer.session_id(),
        expect_sid,
        "should not change peer session id"
    );

    let conn_event = conn_rx.next().await.expect("should have disconnect event");
    match conn_event {
        ConnectionEvent::Disconnect(sid) => assert_eq!(sid, 99.into(), "should be new session id"),
        _ => panic!("should be disconnect event"),
    }
}

#[tokio::test]
async fn should_reject_new_connections_for_same_peer_also_remove_multiaddr_in_unknown_book_on_new_session(
) {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;

    let test_peer = remote_peers.first().expect("get first peer");
    let test_addr = make_multiaddr(999, Some(test_peer.owned_id()));
    mgr.unknown_book_mut()
        .insert(test_addr.clone().try_into().expect("try into addr info"));
    assert_eq!(mgr.unknown_book().len(), 1, "should have one unknown addr");

    let sess_ctx = SessionContext::make(
        SessionId::new(99),
        test_addr,
        SessionType::Outbound,
        test_peer.owned_pubkey(),
    );
    let new_session = PeerManagerEvent::NewSession {
        pid:    test_peer.owned_id(),
        pubkey: test_peer.owned_pubkey(),
        ctx:    sess_ctx.arced(),
    };
    mgr.poll_event(new_session).await;

    let inner = mgr.core_inner();
    assert_eq!(inner.connected(), 1, "should not increase conn count");
    assert_eq!(mgr.unknown_book().len(), 0, "should remove unknown addr");
}

#[tokio::test]
async fn should_keep_new_connection_for_error_outdated_peer_session_on_new_session() {
    let (mut mgr, mut conn_rx) = make_manager(0, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;

    let inner = mgr.core_inner();
    let test_peer = remote_peers.first().expect("get first peer");
    inner.remove_session(test_peer.session_id());

    let sess_ctx = SessionContext::make(
        SessionId::new(99),
        test_peer.raw_multiaddrs().pop().expect("get multiaddr"),
        SessionType::Outbound,
        test_peer.owned_pubkey(),
    );
    let new_session = PeerManagerEvent::NewSession {
        pid:    test_peer.owned_id(),
        pubkey: test_peer.owned_pubkey(),
        ctx:    sess_ctx.arced(),
    };
    mgr.poll_event(new_session).await;

    assert_eq!(inner.connected(), 1, "should not increase conn count");
    assert_eq!(
        test_peer.session_id(),
        99.into(),
        "should update session id"
    );

    match conn_rx.try_next() {
        Err(_) => (), // Err means channel is empty, it's expected
        _ => panic!("should not have any connection event"),
    }
}

#[tokio::test]
async fn should_reject_new_connections_when_we_reach_max_connections_on_new_session() {
    let (mut mgr, mut conn_rx) = make_manager(0, 10); // set max to 10
    let _remote_peers = make_sessions(&mut mgr, 10).await;

    let remote_pubkey = make_pubkey();
    let remote_addr = make_multiaddr(2077, Some(remote_pubkey.peer_id()));
    mgr.unknown_book_mut()
        .insert(make_peer_multiaddr(2077, remote_pubkey.peer_id()).into());

    let sess_ctx = SessionContext::make(
        SessionId::new(99),
        remote_addr,
        SessionType::Outbound,
        remote_pubkey.clone(),
    );
    let new_session = PeerManagerEvent::NewSession {
        pid:    remote_pubkey.peer_id(),
        pubkey: remote_pubkey.clone(),
        ctx:    sess_ctx.arced(),
    };
    mgr.poll_event(new_session).await;

    let inner = mgr.core_inner();
    assert_eq!(inner.connected(), 10, "should not increase conn count");
    assert_eq!(mgr.unknown_book().len(), 1, "should not touch unknown addr");

    let conn_event = conn_rx.next().await.expect("should have disconnect event");
    match conn_event {
        ConnectionEvent::Disconnect(sid) => assert_eq!(sid, 99.into(), "should be new session id"),
        _ => panic!("should be disconnect event"),
    }
}

#[tokio::test]
async fn should_remove_session_on_session_closed() {
    let (mut mgr, _conn_rx) = make_manager(2, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;

    let test_peer = remote_peers.first().expect("get first peer");
    assert_eq!(test_peer.retry(), 0, "should reset retry after connect");
    // Set connected at to older timestamp to increase peer alive
    test_peer.set_connected_at(Peer::now() - ALIVE_RETRY_INTERVAL - 1);

    let session_closed = PeerManagerEvent::SessionClosed {
        pid: test_peer.owned_id(),
        sid: test_peer.session_id(),
    };
    mgr.poll_event(session_closed).await;

    let inner = mgr.core_inner();
    assert_eq!(inner.connected(), 0, "shoulld have zero connected");
    assert_eq!(inner.share_sessions().len(), 0, "should have no session");
    assert_eq!(inner.connecting(), 1, "should have one connecting attempt");
    assert_eq!(
        test_peer.connectedness(),
        Connectedness::Connecting,
        "should try reconnect since we aren't reach max connections"
    );
    assert_eq!(test_peer.retry(), 0, "should keep retry to 0");
}

#[tokio::test]
async fn should_increase_retry_for_short_alive_session_on_session_closed() {
    let (mut mgr, _conn_rx) = make_manager(2, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;

    let test_peer = remote_peers.first().expect("get first peer");
    assert_eq!(test_peer.retry(), 0, "should reset retry after connect");

    let session_closed = PeerManagerEvent::SessionClosed {
        pid: test_peer.owned_id(),
        sid: test_peer.session_id(),
    };
    mgr.poll_event(session_closed).await;

    let inner = mgr.core_inner();
    assert_eq!(
        inner.connected(),
        0,
        "should have no session because of retry"
    );
    assert_eq!(inner.share_sessions().len(), 0, "should have no session");
    assert_eq!(test_peer.connectedness(), Connectedness::CanConnect);
    assert_eq!(test_peer.retry(), 1, "should increase retry count");
}

#[tokio::test]
async fn should_update_peer_alive_on_peer_alive() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;

    let test_peer = remote_peers.first().expect("get first peer");
    let old_alive = test_peer.alive();

    // Set connected at to older timestamp to increase peer alive
    test_peer.set_connected_at(Peer::now() - ALIVE_RETRY_INTERVAL - 1);

    let peer_alive = PeerManagerEvent::PeerAlive {
        pid: test_peer.owned_id(),
    };
    mgr.poll_event(peer_alive).await;

    assert_eq!(
        test_peer.alive(),
        old_alive + ALIVE_RETRY_INTERVAL + 1,
        "should update peer alive"
    );
}

#[tokio::test]
async fn should_reset_peer_retry_on_peer_alive() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;

    let test_peer = remote_peers.first().expect("get first peer");
    assert_eq!(test_peer.retry(), 0, "should have 0 retry");

    test_peer.increase_retry();
    assert_eq!(test_peer.retry(), 1, "should now have 1 retry");

    let peer_alive = PeerManagerEvent::PeerAlive {
        pid: test_peer.owned_id(),
    };
    mgr.poll_event(peer_alive).await;

    assert_eq!(test_peer.retry(), 0, "should reset retry");
}

#[tokio::test]
async fn should_disconnect_session_and_remove_peer_on_remove_peer_by_session() {
    let (mut mgr, mut conn_rx) = make_manager(0, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;

    let test_peer = remote_peers.first().expect("get first peer");
    let expect_sid = test_peer.session_id();
    let remove_peer_by_session = PeerManagerEvent::BadSession {
        sid:  test_peer.session_id(),
        kind: RemoveKind::ProtocolSelect,
    };
    mgr.poll_event(remove_peer_by_session).await;

    let inner = mgr.core_inner();
    assert_eq!(inner.connected(), 0, "should have no session");
    assert_eq!(inner.share_sessions().len(), 0, "should have no session");
    assert!(
        inner.peer(&test_peer.id).is_none(),
        "should remove test peer"
    );

    let conn_event = conn_rx.next().await.expect("should have disconnect event");
    match conn_event {
        ConnectionEvent::Disconnect(sid) => {
            assert_eq!(sid, expect_sid, "should disconnect session")
        }
        _ => panic!("should be disconnect event"),
    }
}

#[tokio::test]
async fn should_keep_bootstrap_peer_but_max_retry_on_remove_peer_by_session() {
    let (mut mgr, mut conn_rx) = make_manager(1, 20);
    let bootstraps = &mgr.config().bootstraps;
    let boot_peer = bootstraps.first().expect("get one bootstrap peer").clone();

    // Insert bootstrap peer
    let inner = mgr.core_inner();
    inner.add_peer(boot_peer.clone());

    // Init bootstrap session
    let sess_ctx = SessionContext::make(
        SessionId::new(1),
        boot_peer.raw_multiaddrs().pop().expect("get multiaddr"),
        SessionType::Outbound,
        boot_peer.owned_pubkey(),
    );
    let new_session = PeerManagerEvent::NewSession {
        pid:    boot_peer.owned_id(),
        pubkey: boot_peer.owned_pubkey(),
        ctx:    sess_ctx.arced(),
    };
    mgr.poll_event(new_session).await;

    assert_eq!(inner.connected(), 1, "should have one session");
    assert_eq!(
        boot_peer.connectedness(),
        Connectedness::Connected,
        "should connecte"
    );
    assert!(
        boot_peer.session_id() != 0.into(),
        "should not be default zero"
    );

    let expect_sid = boot_peer.session_id();
    let remove_peer_by_session = PeerManagerEvent::BadSession {
        sid:  boot_peer.session_id(),
        kind: RemoveKind::ProtocolSelect,
    };
    mgr.poll_event(remove_peer_by_session).await;

    assert_eq!(inner.connected(), 0, "should have no session");
    assert_eq!(inner.share_sessions().len(), 0, "should have no session");
    assert!(
        inner.peer(&boot_peer.id).is_some(),
        "should not remove bootstrap peer"
    );

    assert_eq!(boot_peer.connectedness(), Connectedness::CanConnect);
    assert_eq!(
        boot_peer.retry(),
        MAX_RETRY_COUNT,
        "should set to max retry"
    );

    let conn_event = conn_rx.next().await.expect("should have disconnect event");
    match conn_event {
        ConnectionEvent::Disconnect(sid) => {
            assert_eq!(sid, expect_sid, "should disconnect session")
        }
        _ => panic!("should be disconnect event"),
    }
}

#[tokio::test]
async fn should_mark_session_blocked_on_session_blocked() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;

    let test_peer = remote_peers.first().expect("get first peer");
    let sess_ctx = SessionContext::make(
        test_peer.session_id(),
        test_peer.raw_multiaddrs().pop().expect("get multiaddr"),
        SessionType::Outbound,
        test_peer.owned_pubkey(),
    );
    let session_blocked = PeerManagerEvent::SessionBlocked {
        ctx: sess_ctx.arced(),
    };
    mgr.poll_event(session_blocked).await;

    let inner = mgr.core_inner();
    let session = inner
        .session(test_peer.session_id())
        .expect("should have a session");
    assert!(session.is_blocked(), "should be blocked");
}

#[tokio::test]
async fn should_disconnect_peer_and_increase_retry_on_retry_peer_later() {
    let (mut mgr, mut conn_rx) = make_manager(0, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;

    let test_peer = remote_peers.first().expect("get first peer");
    let expect_sid = test_peer.session_id();
    let retry_peer = PeerManagerEvent::RetryPeerLater {
        pid:  test_peer.owned_id(),
        kind: RetryKind::TimedOut,
    };
    mgr.poll_event(retry_peer).await;

    let inner = mgr.core_inner();
    assert_eq!(inner.connected(), 0, "should have no session");
    assert_eq!(test_peer.connectedness(), Connectedness::CanConnect);
    assert_eq!(test_peer.retry(), 1, "should increase peer retry");

    let conn_event = conn_rx.next().await.expect("should have disconnect event");
    match conn_event {
        ConnectionEvent::Disconnect(sid) => {
            assert_eq!(sid, expect_sid, "should disconnect session")
        }
        _ => panic!("should be disconnect event"),
    }
}

#[tokio::test]
async fn should_try_all_peer_multiaddrs_on_connect_peers_now() {
    let (mut mgr, mut conn_rx) = make_manager(0, 20);
    let peers = (0..10)
        .map(|port| {
            // Every peer has two multiaddrs
            let p = make_peer(port + 7000);
            p.add_multiaddrs(vec![make_peer_multiaddr(port + 8000, p.owned_id())]);
            p
        })
        .collect::<Vec<_>>();

    let inner = mgr.core_inner();
    for peer in peers.iter() {
        inner.add_peer(peer.clone());
    }

    let connect_peers = PeerManagerEvent::ConnectPeersNow {
        pids: peers.iter().map(|p| p.owned_id()).collect(),
    };
    mgr.poll_event(connect_peers).await;

    let conn_event = conn_rx.next().await.expect("should have connect event");
    let multiaddrs_in_event = match conn_event {
        ConnectionEvent::Connect { addrs, .. } => addrs,
        _ => panic!("should be connect event"),
    };

    let expect_multiaddrs = peers
        .into_iter()
        .map(|p| p.raw_multiaddrs())
        .flatten()
        .collect::<Vec<_>>();

    assert_eq!(
        multiaddrs_in_event.len(),
        expect_multiaddrs.len(),
        "should have same number of multiaddrs"
    );
    assert!(
        !multiaddrs_in_event
            .iter()
            .any(|ma| !expect_multiaddrs.contains(ma)),
        "all multiaddrs should be included"
    );
}

#[tokio::test]
async fn should_skip_peers_not_in_can_connect_connectedness_on_connect_peers_now() {
    let (mut mgr, mut conn_rx) = make_manager(0, 20);
    let peer_in_connecting = make_peer(2077);
    let peer_in_connected = make_peer(2020);
    let peer_in_unconnectable = make_peer(2059);

    peer_in_unconnectable.set_connectedness(Connectedness::Unconnectable);
    peer_in_connected.set_connectedness(Connectedness::Connected);
    peer_in_connecting.set_connectedness(Connectedness::Connecting);

    let inner = mgr.core_inner();
    inner.add_peer(peer_in_connecting.clone());
    inner.add_peer(peer_in_connected.clone());
    inner.add_peer(peer_in_unconnectable.clone());

    let connect_peers = PeerManagerEvent::ConnectPeersNow {
        pids: vec![
            peer_in_unconnectable.owned_id(),
            peer_in_connected.owned_id(),
            peer_in_connecting.owned_id(),
        ],
    };
    mgr.poll_event(connect_peers).await;

    match conn_rx.try_next() {
        Err(_) => (), // Err means channel is empty, it's expected
        _ => panic!("should not have any connection event"),
    }
}

#[tokio::test]
async fn should_connect_peers_even_if_they_are_not_retry_ready_on_connect_peers_now() {
    let (mut mgr, mut conn_rx) = make_manager(0, 20);
    let not_ready_peer = make_peer(2077);
    not_ready_peer.increase_retry();

    let inner = mgr.core_inner();
    inner.add_peer(not_ready_peer.clone());

    let connect_peers = PeerManagerEvent::ConnectPeersNow {
        pids: vec![not_ready_peer.owned_id()],
    };
    mgr.poll_event(connect_peers).await;

    let conn_event = conn_rx.next().await.expect("should have connect event");
    let multiaddrs_in_event = match conn_event {
        ConnectionEvent::Connect { addrs, .. } => addrs,
        _ => panic!("should be connect event"),
    };

    let expect_multiaddrs = not_ready_peer.raw_multiaddrs();
    assert_eq!(
        multiaddrs_in_event.len(),
        expect_multiaddrs.len(),
        "should have same number of multiaddrs"
    );
    assert!(
        !multiaddrs_in_event
            .iter()
            .any(|ma| !expect_multiaddrs.contains(ma)),
        "all multiaddrs should be included"
    );
}

// DiscoverMultiAddrs reuse DiscoverAddr logic
#[tokio::test]
async fn should_add_multiaddrs_to_unknown_book_on_discover_multi_addrs() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let test_multiaddrs: Vec<Multiaddr> = (0..10)
        .map(|port| {
            let peer = make_peer(port + 7000);
            peer.raw_multiaddrs().pop().expect("peer multiaddr")
        })
        .collect();

    assert!(mgr.unknown_book().is_empty());

    let discover_multi_addrs = PeerManagerEvent::DiscoverMultiAddrs {
        addrs: test_multiaddrs.clone(),
    };
    mgr.poll_event(discover_multi_addrs).await;

    let unknown_book = &mgr.inner.unknown_addrs;
    assert_eq!(unknown_book.len(), 10, "should have 10 multiaddrs");
    assert!(
        !test_multiaddrs.iter().any(|ma| !unknown_book.contains(ma)),
        "test multiaddrs should all be inserted"
    );
}

#[tokio::test]
async fn should_skip_our_listen_multiaddrs_on_discover_multi_addrs() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let self_id = mgr.inner.peer_id.to_owned();

    let inner = mgr.core_inner();
    let listen_multiaddr = make_peer_multiaddr(2020, self_id.clone());

    inner.add_listen(listen_multiaddr.clone());
    assert!(
        inner.listen().contains(&listen_multiaddr),
        "should contains listen addr"
    );

    let discover_multi_addrs = PeerManagerEvent::DiscoverMultiAddrs {
        addrs: vec![make_multiaddr(2020, Some(self_id.clone()))],
    };
    mgr.poll_event(discover_multi_addrs).await;

    assert!(
        mgr.unknown_book().is_empty(),
        "should not add our listen addr to unknown"
    );
}

#[tokio::test]
async fn should_skip_already_exist_peer_multiaddr_on_discover_multi_addrs() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;
    let test_peer = remote_peers.first().expect("get first");

    assert!(mgr.unknown_book().is_empty());

    let discover_multi_addrs = PeerManagerEvent::DiscoverMultiAddrs {
        addrs: vec![test_peer.raw_multiaddrs().pop().expect("peer multiaddr")],
    };
    mgr.poll_event(discover_multi_addrs).await;

    assert!(
        mgr.unknown_book().is_empty(),
        "should not add exist peer's multiaddr"
    );
}

#[tokio::test]
async fn should_skip_multiaddrs_without_id_included_on_discover_multi_addrs() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let test_multiaddr = make_multiaddr(2077, None);

    assert!(mgr.unknown_book().is_empty());
    let discover_multi_addrs = PeerManagerEvent::DiscoverMultiAddrs {
        addrs: vec![test_multiaddr],
    };
    mgr.poll_event(discover_multi_addrs).await;

    assert!(
        mgr.unknown_book().is_empty(),
        "should ignore multiaddr without id included"
    );
}

#[tokio::test]
async fn should_add_multiaddrs_to_peer_on_identified_addrs() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;
    let test_peer = remote_peers.first().expect("get first");
    let old_multiaddrs_len = test_peer.multiaddrs_len();

    let test_multiaddrs: Vec<_> = (0..2)
        .map(|port| make_multiaddr(port + 9000, Some(test_peer.owned_id())))
        .collect();

    let identified_addrs = PeerManagerEvent::IdentifiedAddrs {
        pid:   test_peer.owned_id(),
        addrs: test_multiaddrs.clone(),
    };
    mgr.poll_event(identified_addrs).await;

    assert_eq!(
        test_peer.multiaddrs_len(),
        old_multiaddrs_len + 2,
        "should have correct multiaddrs len"
    );
    assert!(
        !test_multiaddrs
            .iter()
            .any(|ma| !test_peer.raw_multiaddrs().contains(ma)),
        "should add all multiaddrs to peer"
    );
}

#[tokio::test]
async fn should_push_id_to_multiaddrs_if_not_included_on_identified_addrs() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;
    let test_peer = remote_peers.first().expect("get first");
    let test_multiaddr = make_multiaddr(2077, None);

    let identified_addrs = PeerManagerEvent::IdentifiedAddrs {
        pid:   test_peer.owned_id(),
        addrs: vec![test_multiaddr.clone()],
    };
    mgr.poll_event(identified_addrs).await;

    assert!(
        !test_peer.raw_multiaddrs().contains(&test_multiaddr),
        "should not contain multiaddr without id included"
    );

    let with_id = make_multiaddr(2077, Some(test_peer.owned_id()));
    assert!(
        test_peer.raw_multiaddrs().contains(&with_id),
        "should push id to multiaddr when add it to peer"
    );
}

#[tokio::test]
async fn should_add_dialer_multiaddr_to_peer_on_repeated_connection() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;
    let test_peer = remote_peers.first().expect("get first");
    let test_multiaddr = make_multiaddr(2077, Some(test_peer.owned_id()));

    let repeated_connection = PeerManagerEvent::RepeatedConnection {
        ty:   ConnectionType::Outbound,
        sid:  test_peer.session_id(),
        addr: test_multiaddr.clone(),
    };
    mgr.poll_event(repeated_connection).await;

    assert!(
        test_peer.raw_multiaddrs().contains(&test_multiaddr),
        "should add dialer multiaddr to peer"
    );
}

#[tokio::test]
async fn should_skip_listen_multiaddr_to_peer_on_repeated_connection() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;
    let test_peer = remote_peers.first().expect("get first");
    let test_multiaddr = make_multiaddr(2077, Some(test_peer.owned_id()));

    let repeated_connection = PeerManagerEvent::RepeatedConnection {
        ty:   ConnectionType::Inbound,
        sid:  test_peer.session_id(),
        addr: test_multiaddr.clone(),
    };
    mgr.poll_event(repeated_connection).await;

    assert!(
        !test_peer.raw_multiaddrs().contains(&test_multiaddr),
        "should skip listen multiaddr to peer"
    );
}

#[tokio::test]
async fn should_push_id_if_multiaddr_not_included_on_repeated_connection() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;
    let test_peer = remote_peers.first().expect("get first");
    let test_multiaddr = make_multiaddr(2077, None);

    let repeated_connection = PeerManagerEvent::RepeatedConnection {
        ty:   ConnectionType::Outbound,
        sid:  test_peer.session_id(),
        addr: test_multiaddr.clone(),
    };
    mgr.poll_event(repeated_connection).await;

    assert!(
        !test_peer.raw_multiaddrs().contains(&test_multiaddr),
        "should not add multiaddr without id included"
    );

    let with_id = make_multiaddr(2077, Some(test_peer.owned_id()));
    assert!(
        test_peer.raw_multiaddrs().contains(&with_id),
        "should add multiaddr wit id included"
    );
}

#[tokio::test]
async fn should_remove_multiaddr_in_unknown_book_on_repeated_connection() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;
    let test_peer = remote_peers.first().expect("get first");
    let test_multiaddr = make_multiaddr(2077, Some(test_peer.owned_id()));

    mgr.unknown_book_mut()
        .insert(make_peer_multiaddr(2077, test_peer.owned_id()).into());
    assert_eq!(
        mgr.unknown_book().len(),
        1,
        "should have 1 unknown multiaddrs"
    );

    let repeated_connection = PeerManagerEvent::RepeatedConnection {
        ty:   ConnectionType::Outbound,
        sid:  test_peer.session_id(),
        addr: test_multiaddr,
    };
    mgr.poll_event(repeated_connection).await;

    assert!(
        mgr.unknown_book().is_empty(),
        "should remove multiaddrs in unknown book"
    );
}

#[tokio::test]
async fn should_remove_multiaddr_in_unknown_book_if_event_multiaddr_doesnt_have_id_on_repeated_connection(
) {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;
    let test_peer = remote_peers.first().expect("get first");
    let test_multiaddr = make_multiaddr(2077, None);

    mgr.unknown_book_mut()
        .insert(make_peer_multiaddr(2077, test_peer.owned_id()).into());
    assert_eq!(
        mgr.unknown_book().len(),
        1,
        "should have 1 unknown multiaddrs"
    );

    let repeated_connection = PeerManagerEvent::RepeatedConnection {
        ty:   ConnectionType::Outbound,
        sid:  test_peer.session_id(),
        addr: test_multiaddr,
    };
    mgr.poll_event(repeated_connection).await;

    assert!(
        mgr.unknown_book().is_empty(),
        "should remove multiaddrs in unknown book"
    );
}

#[tokio::test]
async fn should_always_remove_inbound_multiaddr_in_unknown_book_on_repeated_connection() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;
    let test_peer = remote_peers.first().expect("get first");
    let test_multiaddr = make_multiaddr(2077, None);

    mgr.unknown_book_mut()
        .insert(make_peer_multiaddr(2077, test_peer.owned_id()).into());
    assert_eq!(
        mgr.unknown_book().len(),
        1,
        "should have 1 unknown multiaddrs"
    );

    let repeated_connection = PeerManagerEvent::RepeatedConnection {
        ty:   ConnectionType::Inbound,
        sid:  test_peer.session_id(),
        addr: test_multiaddr,
    };
    mgr.poll_event(repeated_connection).await;

    assert!(
        mgr.unknown_book().is_empty(),
        "should remove multiaddrs in unknown book"
    );
}

#[tokio::test]
async fn should_remove_multiaddr_in_unknown_book_on_unconnectable_multiaddr() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let test_peer = make_peer(2077);
    let test_multiaddr = test_peer.raw_multiaddrs().pop().expect("peer multiaddr");

    mgr.unknown_book_mut().insert(
        test_multiaddr
            .clone()
            .try_into()
            .expect("try into addr info"),
    );
    assert_eq!(
        mgr.unknown_book().len(),
        1,
        "should have 1 multiaddr in unknown book"
    );

    let unconnectable_multiaddr = PeerManagerEvent::UnconnectableAddress {
        addr: test_multiaddr,
        kind: RemoveKind::ProtocolSelect,
    };
    mgr.poll_event(unconnectable_multiaddr).await;

    assert!(mgr.unknown_book().is_empty());
}

#[tokio::test]
async fn should_remove_peer_multiaddr_and_increase_peer_retry_on_unconnectable_multiaddr() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let test_peer = make_peer(2077);
    let test_multiaddr = test_peer.raw_multiaddrs().pop().expect("peer multiaddr");

    let inner = mgr.core_inner();
    inner.add_peer(test_peer.clone());
    assert_eq!(test_peer.retry(), 0, "should have 0 retry");

    let unconnectable_multiaddr = PeerManagerEvent::UnconnectableAddress {
        addr: test_multiaddr,
        kind: RemoveKind::ProtocolSelect,
    };
    mgr.poll_event(unconnectable_multiaddr).await;

    assert_eq!(
        test_peer.multiaddrs_len(),
        0,
        "should remove this unconnectable multiaddr from peer"
    );
    assert_eq!(test_peer.retry(), 1, "should increase peer retry");
}

#[tokio::test]
async fn should_not_remove_bootstrap_mutiaddr_on_unconnectable_multiaddr() {
    let (mut mgr, _conn_rx) = make_manager(1, 20);
    let bootstraps = &mgr.config().bootstraps;
    let boot_peer = bootstraps.first().expect("get one bootstrap peer").clone();
    let boot_multiaddr = boot_peer.raw_multiaddrs().pop().expect("boot multiaddr");
    let old_multiaddrs_len = boot_peer.multiaddrs_len();

    // Insert bootstrap peer
    let inner = mgr.core_inner();
    inner.add_peer(boot_peer.clone());
    assert_eq!(boot_peer.retry(), 0, "should have 0 retry");

    let unconnectable_multiaddr = PeerManagerEvent::UnconnectableAddress {
        addr: boot_multiaddr,
        kind: RemoveKind::ProtocolSelect,
    };
    mgr.poll_event(unconnectable_multiaddr).await;

    assert_eq!(
        boot_peer.multiaddrs_len(),
        old_multiaddrs_len,
        "should not remove boot multiaddr"
    );
    assert_eq!(boot_peer.retry(), 1, "should increase boot retry");
}

#[tokio::test]
async fn should_increase_retry_for_multiaddr_in_unknown_on_reconnect_addr_later() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let test_peer = make_peer(2077);
    let test_multiaddr = test_peer.raw_multiaddrs().pop().expect("peer multiaddr");

    mgr.unknown_book_mut().insert(
        test_multiaddr
            .clone()
            .try_into()
            .expect("try into addr info"),
    );
    assert_eq!(
        mgr.unknown_book().len(),
        1,
        "should have 1 multiaddr in unknown book"
    );

    let reconnect_addr_later = PeerManagerEvent::ReconnectAddrLater {
        addr: test_multiaddr.clone(),
        kind: RetryKind::TimedOut,
    };
    mgr.poll_event(reconnect_addr_later).await;

    let addr_in_unknown_book = mgr
        .inner
        .unknown_addrs
        .get(&test_multiaddr)
        .expect("get addr from unknown book");
    assert_eq!(addr_in_unknown_book.retry(), 1, "should increase retry");
}

#[tokio::test]
async fn should_remove_multiaddr_from_unknown_book_when_run_out_retry_on_reconnect_addr_later() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let test_peer = make_peer(2077);
    let test_multiaddr = test_peer.raw_multiaddrs().pop().expect("peer multiaddr");

    mgr.inner.unknown_addrs.insert(
        test_multiaddr
            .clone()
            .try_into()
            .expect("try into addr info"),
    );
    let addr_in_unknown_book = mgr
        .inner
        .unknown_addrs
        .get(&test_multiaddr)
        .expect("get addr from unknown book")
        .clone();

    addr_in_unknown_book.inc_retry_by(MAX_ADDR_RETRY);

    let reconnect_addr_later = PeerManagerEvent::ReconnectAddrLater {
        addr: test_multiaddr.clone(),
        kind: RetryKind::TimedOut,
    };
    mgr.poll_event(reconnect_addr_later).await;

    assert!(addr_in_unknown_book.run_out_retry(), "should run out retry");
    assert!(
        mgr.unknown_book().is_empty(),
        "should be remove from unknown book"
    );
}

#[tokio::test]
async fn should_set_connectedness_and_increase_peer_retry_on_reconnect_addr_later() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let test_peer = make_peer(2077);
    let test_multiaddr = test_peer.raw_multiaddrs().pop().expect("peer multiaddr");

    let inner = mgr.core_inner();
    inner.add_peer(test_peer.clone());
    assert_eq!(test_peer.retry(), 0, "should have 0 retry");

    let reconnect_addr_later = PeerManagerEvent::ReconnectAddrLater {
        addr: test_multiaddr.clone(),
        kind: RetryKind::TimedOut,
    };
    mgr.poll_event(reconnect_addr_later).await;

    assert_eq!(test_peer.connectedness(), Connectedness::CanConnect);
    assert_eq!(test_peer.retry(), 1, "should increase peer retry");
}

#[tokio::test]
async fn should_skip_on_connected_peer_on_reconnect_addr_later() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let remote_peers = make_sessions(&mut mgr, 1).await;
    let test_peer = remote_peers.first().expect("get first");
    let test_multiaddr = make_multiaddr(2077, None);

    assert_eq!(test_peer.retry(), 0, "should have 0 retry");

    let reconnect_addr_later = PeerManagerEvent::ReconnectAddrLater {
        addr: test_multiaddr.clone(),
        kind: RetryKind::TimedOut,
    };
    mgr.poll_event(reconnect_addr_later).await;

    assert_eq!(test_peer.connectedness(), Connectedness::Connected);
    assert_eq!(test_peer.retry(), 0, "should have not increase retry");
}

#[tokio::test]
async fn should_remove_peer_when_it_run_out_of_retry_on_reconnect_addr_later() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let test_peer = make_peer(2077);
    let test_multiaddr = test_peer.raw_multiaddrs().pop().expect("peer multiaddr");

    let inner = mgr.core_inner();
    inner.add_peer(test_peer.clone());

    test_peer.set_retry(MAX_RETRY_COUNT);
    assert_eq!(
        test_peer.retry(),
        MAX_RETRY_COUNT,
        "should have reach max retry"
    );
    // Set to older timestamp, so that we can increase retry
    test_peer.set_attempt_at(Peer::now() - VALID_ATTEMPT_INTERVAL - 1);

    let reconnect_addr_later = PeerManagerEvent::ReconnectAddrLater {
        addr: test_multiaddr.clone(),
        kind: RetryKind::TimedOut,
    };
    mgr.poll_event(reconnect_addr_later).await;

    assert!(inner.peer(&test_peer.id).is_none(), "should remove peer");
}

#[tokio::test]
async fn should_reset_bootstrap_retry_when_it_run_out_of_retry_on_reconnect_addr_later() {
    let (mut mgr, _conn_rx) = make_manager(1, 20);
    let bootstraps = &mgr.config().bootstraps;
    let boot_peer = bootstraps.first().expect("get one bootstrap peer").clone();
    let boot_multiaddr = boot_peer.raw_multiaddrs().pop().expect("boot multiaddr");

    // Insert bootstrap peer
    let inner = mgr.core_inner();
    inner.add_peer(boot_peer.clone());

    boot_peer.set_retry(MAX_RETRY_COUNT);
    assert_eq!(
        boot_peer.retry(),
        MAX_RETRY_COUNT,
        "should have reach max retry"
    );
    // Set to older timestamp, so that we can increase retry
    boot_peer.set_attempt_at(Peer::now() - VALID_ATTEMPT_INTERVAL - 1);

    let reconnect_addr_later = PeerManagerEvent::ReconnectAddrLater {
        addr: boot_multiaddr.clone(),
        kind: RetryKind::TimedOut,
    };
    mgr.poll_event(reconnect_addr_later).await;

    assert!(
        inner.peer(&boot_peer.id).is_some(),
        "should not remove bootstrap peer"
    );
    assert_eq!(boot_peer.retry(), 0, "should reset bootstrap retry");
}

#[tokio::test]
async fn should_reset_protected_peer_retry_when_it_run_out_of_retry_on_reconnect_addr_later() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let test_peer = make_peer(2077);
    let test_multiaddr = test_peer.raw_multiaddrs().pop().expect("peer multiaddr");

    let inner = mgr.core_inner();
    inner.add_peer(test_peer.clone());
    inner.protect_peers_by_chain_addr(vec![test_peer.chain_addr.as_ref().to_owned()]);
    assert_eq!(inner.whitelist().len(), 1, "should have one whitelisted");

    test_peer.set_retry(MAX_RETRY_COUNT);
    assert_eq!(
        test_peer.retry(),
        MAX_RETRY_COUNT,
        "should have reach max retry"
    );
    // Set to older timestamp, so that we can increase retry
    test_peer.set_attempt_at(Peer::now() - VALID_ATTEMPT_INTERVAL - 1);

    let reconnect_addr_later = PeerManagerEvent::ReconnectAddrLater {
        addr: test_multiaddr.clone(),
        kind: RetryKind::TimedOut,
    };
    mgr.poll_event(reconnect_addr_later).await;

    assert!(
        inner.peer(&test_peer.id).is_some(),
        "should not remove protected peer"
    );
    assert_eq!(test_peer.retry(), 0, "should reset protected peer's retry");
}

#[tokio::test]
async fn should_add_new_listen_on_add_new_listen_addr() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let self_id = mgr.inner.peer_id.to_owned();

    let inner = mgr.core_inner();
    let listen_multiaddr = make_peer_multiaddr(2020, self_id.clone());
    inner.add_listen(listen_multiaddr.clone());
    assert!(!inner.listen().is_empty(), "should have listen address");

    let test_multiaddr = make_multiaddr(2077, Some(self_id));
    assert!(test_multiaddr != *listen_multiaddr);

    let add_listen_addr = PeerManagerEvent::AddNewListenAddr {
        addr: test_multiaddr.clone(),
    };
    mgr.poll_event(add_listen_addr).await;

    assert_eq!(inner.listen().len(), 2, "should have 2 listen addrs");
    assert!(
        inner.listen().contains(&test_multiaddr),
        "should add new listen multiaddr"
    );
}

#[tokio::test]
async fn should_push_id_to_listen_multiaddr_if_not_included_on_add_new_listen_addr() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let self_id = mgr.inner.peer_id.to_owned();

    let inner = mgr.core_inner();
    let test_multiaddr = make_multiaddr(2077, None);
    assert!(inner.listen().is_empty(), "should not have any listen addr");

    let add_listen_addr = PeerManagerEvent::AddNewListenAddr {
        addr: test_multiaddr.clone(),
    };
    mgr.poll_event(add_listen_addr).await;

    let with_id = make_multiaddr(2077, Some(self_id));
    assert_eq!(inner.listen().len(), 1, "should have one listen addr");
    assert!(
        inner.listen().contains(&with_id),
        "should add new listen multiaddr"
    );
}

#[tokio::test]
async fn should_remove_new_listen_multiaddr_in_unknown_book_on_add_new_listen_addr() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let self_id = mgr.inner.peer_id.to_owned();

    let test_multiaddr = make_peer_multiaddr(2077, self_id);
    mgr.unknown_book_mut().insert(test_multiaddr.clone().into());
    assert_eq!(mgr.unknown_book().len(), 1, "should have one unknown addr");

    let inner = mgr.core_inner();
    assert!(inner.listen().is_empty(), "should not have any listen addr");

    let add_listen_addr = PeerManagerEvent::AddNewListenAddr {
        addr: test_multiaddr.into(),
    };
    mgr.poll_event(add_listen_addr).await;

    assert_eq!(
        mgr.unknown_book().len(),
        0,
        "should remove new listen addr in unknown book"
    );
}

#[tokio::test]
async fn should_remove_listen_on_remove_listen_addr() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let self_id = mgr.inner.peer_id.to_owned();

    let inner = mgr.core_inner();
    let listen_multiaddr = make_peer_multiaddr(2020, self_id.clone());

    inner.add_listen(listen_multiaddr.clone());
    assert!(
        inner.listen().contains(&listen_multiaddr),
        "should contains listen addr"
    );

    let remove_listen_addr = PeerManagerEvent::RemoveListenAddr {
        addr: make_multiaddr(2020, Some(self_id)),
    };
    mgr.poll_event(remove_listen_addr).await;

    assert_eq!(inner.listen().len(), 0, "should have 0 listen addrs");
}

#[tokio::test]
async fn should_remove_listen_even_if_no_peer_id_included_on_remove_listen_addr() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let self_id = mgr.inner.peer_id.to_owned();

    let inner = mgr.core_inner();
    let listen_multiaddr = make_peer_multiaddr(2020, self_id.clone());

    inner.add_listen(listen_multiaddr.clone());
    assert!(
        inner.listen().contains(&listen_multiaddr),
        "should contains listen addr"
    );

    let remove_listen_addr = PeerManagerEvent::RemoveListenAddr {
        addr: make_multiaddr(2020, None),
    };
    mgr.poll_event(remove_listen_addr).await;

    assert_eq!(inner.listen().len(), 0, "should have 0 listen addrs");
}

#[tokio::test]
async fn should_always_include_our_listen_addrs_in_return_from_manager_handle_random_addrs() {
    let (mgr, _conn_rx) = make_manager(0, 20);
    let self_id = mgr.inner.peer_id.to_owned();

    let inner = mgr.core_inner();
    let listen_multiaddrs = (0..5)
        .map(|port| make_peer_multiaddr(port + 9000, self_id.clone()))
        .collect::<Vec<_>>();

    for ma in listen_multiaddrs.iter() {
        inner.add_listen(ma.clone());
    }

    let handle = mgr.inner.handle();
    let addrs = handle.random_addrs(100);

    assert!(
        !listen_multiaddrs.iter().any(|lma| !addrs.contains(&*lma)),
        "should include our listen addresses"
    );
}

#[tokio::test]
async fn should_increase_retry_for_connecting_timeout_multiaddr_in_unknown_book() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);

    let remote_pubkey = make_pubkey();
    mgr.unknown_book_mut()
        .insert(make_peer_multiaddr(9527, remote_pubkey.peer_id()).into());
    assert_eq!(mgr.unknown_book().len(), 1, "should have one unknown addr");

    let test_addr = mgr
        .unknown_book()
        .iter()
        .next()
        .expect("get one unknown addr")
        .clone();
    assert_eq!(test_addr.retry(), 0, "should have 0 retry");

    test_addr.mark_connecting();
    // Set attempt at to oder timestamp, so that we have connecting timeout
    test_addr.set_attempt_at(AddrInfo::now() - ADDR_TIMEOUT - 1);

    mgr.poll().await;
    assert_eq!(test_addr.retry(), 1, "should have 1 retry");
    assert!(!test_addr.is_connecting(), "should reset connecting flag");
}

#[tokio::test]
async fn should_remove_timeout_connecting_multiaddr_when_run_out_retry_in_unknown_book() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);

    let remote_pubkey = make_pubkey();
    mgr.unknown_book_mut()
        .insert(make_peer_multiaddr(9527, remote_pubkey.peer_id()).into());
    assert_eq!(mgr.unknown_book().len(), 1, "should have one unknown addr");

    let test_addr = mgr
        .unknown_book()
        .iter()
        .next()
        .expect("get one unknown addr")
        .clone();
    assert_eq!(test_addr.retry(), 0, "should have 0 retry");

    test_addr.inc_retry_by(MAX_ADDR_RETRY);
    assert_eq!(test_addr.retry(), MAX_ADDR_RETRY, "should reach max retry");

    test_addr.mark_connecting();
    // Set attempt at to oder timestamp, so that we have connecting timeout
    test_addr.set_attempt_at(AddrInfo::now() - ADDR_TIMEOUT - 1);

    mgr.poll().await;
    assert_eq!(mgr.unknown_book().len(), 0, "should not have any address");
}

#[tokio::test]
async fn should_whitelist_peer_chain_addrs_on_protect_peers_by_chain_addrs() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let peers = (0..5)
        .map(|port| make_peer(port + 9000))
        .collect::<Vec<_>>();
    let chain_addrs = peers
        .into_iter()
        .map(|p| p.chain_addr.as_ref().to_owned())
        .collect::<Vec<_>>();

    let inner = mgr.core_inner();
    assert!(inner.whitelist().is_empty(), "should have empty whitelist");

    let protect_peers_by_chain_addrs = PeerManagerEvent::ProtectPeersByChainAddr {
        chain_addrs: chain_addrs.clone(),
    };
    mgr.poll_event(protect_peers_by_chain_addrs).await;

    let whitelist = inner.whitelist();
    assert_eq!(
        whitelist.len(),
        chain_addrs.len(),
        "should have chain addrs"
    );
    assert!(
        !chain_addrs.into_iter().any(|ca| !whitelist.contains(&ca)),
        "should add all chain addrs"
    );
}

#[tokio::test]
async fn should_allow_whitelisted_peer_session_even_if_we_reach_max_connections_on_new_session() {
    let (mut mgr, _conn_rx) = make_manager(0, 10);
    let _remote_peers = make_sessions(&mut mgr, 10).await;

    let whitelisted_peer = make_peer(2077);
    let peer = make_peer(2019);

    let inner = mgr.core_inner();
    inner.protect_peers_by_chain_addr(vec![whitelisted_peer.chain_addr.as_ref().to_owned()]);

    assert_eq!(inner.whitelist().len(), 1, "should have one whitelisted");
    assert_eq!(inner.connected(), 10, "should have 10 connections");

    // First no whitelisted one
    let sess_ctx = SessionContext::make(
        SessionId::new(233),
        peer.raw_multiaddrs().pop().expect("peer multiaddr"),
        SessionType::Inbound,
        peer.owned_pubkey(),
    );
    let new_session = PeerManagerEvent::NewSession {
        pid:    peer.owned_id(),
        pubkey: peer.owned_pubkey(),
        ctx:    sess_ctx.arced(),
    };
    mgr.poll_event(new_session).await;

    assert_eq!(inner.connected(), 10, "should remain 10 connections");

    // Now whitelistd one
    let sess_ctx = SessionContext::make(
        SessionId::new(666),
        whitelisted_peer
            .raw_multiaddrs()
            .pop()
            .expect("peer multiaddr"),
        SessionType::Inbound,
        whitelisted_peer.owned_pubkey(),
    );
    let new_session = PeerManagerEvent::NewSession {
        pid:    whitelisted_peer.owned_id(),
        pubkey: whitelisted_peer.owned_pubkey(),
        ctx:    sess_ctx.arced(),
    };
    mgr.poll_event(new_session).await;

    assert_eq!(inner.connected(), 11, "should remain 11 connections");
    let session = inner.session(666.into()).expect("should have session");
    assert_eq!(
        session.peer.id, whitelisted_peer.id,
        "should be whitelisted peer"
    );
}

#[tokio::test]
async fn should_refresh_whitelist_on_protect_peers_by_chain_addrs() {
    let (mut mgr, _conn_rx) = make_manager(0, 10);
    let peer = make_peer(2077);

    let inner = mgr.core_inner();
    inner.protect_peers_by_chain_addr(vec![peer.chain_addr.as_ref().to_owned()]);
    assert_eq!(inner.whitelist().len(), 1, "should have one whitelisted");

    let peer_in_list = inner
        .whitelist()
        .iter()
        .next()
        .expect("should have one whitelist peer")
        .clone();
    // Set authorized_at to older timestamp
    peer_in_list.set_authorized_at(ArcProtectedPeer::now() - WHITELIST_TIMEOUT + 20);
    assert!(!peer_in_list.is_expired(), "should not be expired");

    let protect_peers_by_chain_addrs = PeerManagerEvent::ProtectPeersByChainAddr {
        chain_addrs: vec![peer.chain_addr.as_ref().to_owned()],
    };
    mgr.poll_event(protect_peers_by_chain_addrs).await;

    assert_eq!(
        peer_in_list.authorized_at(),
        ArcProtectedPeer::now(),
        "should be refreshed"
    );
}

#[tokio::test]
async fn should_remove_expired_peers_in_whitelist() {
    let (mut mgr, _conn_rx) = make_manager(0, 10);
    let peer = make_peer(2077);

    let inner = mgr.core_inner();
    inner.protect_peers_by_chain_addr(vec![peer.chain_addr.as_ref().to_owned()]);
    assert_eq!(inner.whitelist().len(), 1, "should have one whitelisted");

    let peer_in_list = inner
        .whitelist()
        .iter()
        .next()
        .expect("should have one whitelist peer")
        .clone();
    // Set authorized_at to older timestamp
    peer_in_list.set_authorized_at(ArcProtectedPeer::now() - WHITELIST_TIMEOUT - 1);
    assert!(peer_in_list.is_expired(), "should be expired");

    mgr.poll().await;
    assert_eq!(
        inner.whitelist().len(),
        0,
        "should remove expired peers in whitelist"
    );
}

#[tokio::test]
async fn should_count_connecting_and_connected() {
    let (mut mgr, _conn_rx) = make_manager(0, 20);
    let _remote_peers = make_sessions(&mut mgr, 5).await; // 5 connected peers

    let inner = mgr.core_inner();
    assert_eq!(inner.connected(), 5, "should have 5 connected peers");

    // Register peers
    let peers = (0..5)
        .map(|port| make_peer(port + 5000))
        .collect::<Vec<_>>();
    for peer in peers.iter() {
        inner.add_peer(peer.clone());
    }
    assert!(
        !peers
            .iter()
            .any(|p| p.connectedness() != Connectedness::CanConnect),
        "should all be CanConnect"
    );

    mgr.poll().await;
    assert_eq!(inner.connected(), 5, "should still have 5 connected peers");
    assert_eq!(inner.connecting(), 5, "should have 5 connecting peers");
    assert!(
        !peers
            .iter()
            .any(|p| p.connectedness() != Connectedness::Connecting),
        "should all be Connecting"
    );
}

#[tokio::test]
async fn should_dec_connecting_and_update_peer_state_when_connecting_peer_is_rejected_due_to_max_connections_on_new_session(
) {
    let (mut mgr, _conn_rx) = make_manager(0, 2);

    let inner = mgr.core_inner();
    let peers = (0..3)
        .map(|port| make_peer(port + 5000))
        .collect::<Vec<_>>();

    for peer in peers.iter() {
        inner.add_peer(peer.clone());
    }
    mgr.poll().await;
    assert_eq!(inner.connecting(), 3, "should have 3 connecting attempts");

    for (idx, peer) in peers.iter().enumerate() {
        let sess_ctx = SessionContext::make(
            SessionId::new(idx + 1),
            peer.raw_multiaddrs().pop().expect("get multiaddr"),
            SessionType::Outbound,
            peer.owned_pubkey(),
        );
        let new_session = PeerManagerEvent::NewSession {
            pid:    peer.owned_id(),
            pubkey: peer.owned_pubkey(),
            ctx:    sess_ctx.arced(),
        };
        mgr.poll_event(new_session).await;
    }

    // Since we set max connections to 2, only first two sessions are accepted
    assert_eq!(inner.connected(), 2, "should have 2 connections");
    assert_eq!(inner.connecting(), 0, "should have 0 connecting attempt");

    let available_peers = peers
        .iter()
        .filter(|p| p.connectedness() == Connectedness::CanConnect)
        .count();
    assert_eq!(
        available_peers, 1,
        "should have 1 peer available for connecting"
    );
}

#[tokio::test]
async fn should_dec_connecting_for_connecting_peer_on_retry_peer_ater() {
    let (mut mgr, _conn_rx) = make_manager(0, 5);

    let inner = mgr.core_inner();
    let peer = make_peer(9527);
    inner.add_peer(peer.clone());
    mgr.poll().await;
    assert_eq!(inner.connecting(), 1, "should have one connecting attempt");

    let retry_peer = PeerManagerEvent::RetryPeerLater {
        pid:  peer.owned_id(),
        kind: RetryKind::TimedOut,
    };
    mgr.poll_event(retry_peer).await;

    assert_eq!(inner.connecting(), 0, "should have 0 connecting attempt");
}