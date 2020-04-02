use async_trait::async_trait;
use tokio::sync::{RwLock, Mutex};
use muta_protocol::types::Address;
use wormhole::{
    crypto::{PeerId, PublicKey},
    multiaddr::Multiaddr,
    network::Connectedness,
};

use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    ops::Deref,
    sync::Arc,
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Debug)]
pub struct Peer {
    pub id: PeerId,
    pubkey: Mutex<Option<PublicKey>>,
    chain_addr: Mutex<Option<Address>>,
    multiaddrs: RwLock<HashSet<Multiaddr>>,
    connectedness: AtomicUsize,
}

impl Peer {
    pub fn new(id: PeerId) -> Self {
        Peer {
            id,
            pubkey: Mutex::new(None),
            chain_addr: Mutex::new(None),
            multiaddrs: RwLock::new(Default::default()),
            connectedness: AtomicUsize::new(Connectedness::NotConnected.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ArcPeer(Arc<Peer>);

impl ArcPeer {
    pub fn new(id: PeerId) -> ArcPeer {
        ArcPeer(Arc::new(Peer::new(id)))
    }

    pub async fn set_multiaddrs(&self, addrs: Vec<Multiaddr>) {
        // TODO: check peer id
        self.multiaddrs.write().await.extend(addrs)
    }

    pub async fn add_multiaddr(&self, addr: Multiaddr) {
        // TODO: check peer id
        self.multiaddrs.write().await.insert(addr);
    }

    pub async fn multiaddrs(&self) -> Vec<Multiaddr> {
        self.multiaddrs.read().await.iter().cloned().collect()
    }

    pub async fn set_pubkey(&self, pubkey: PublicKey) {
        let chain_addr = Address::from_pubkey_bytes(pubkey.to_bytes()).expect("chain address from pubkey");

        // TODO: check peer id
        *self.pubkey.lock().await = Some(pubkey);
        *self.chain_addr.lock().await = Some(chain_addr);
    }

    pub async fn pubkey(&self) -> Option<PublicKey> {
        self.pubkey.lock().await.clone()
    }

    pub async fn chain_addr(&self) -> Option<Address> {
        self.chain_addr.lock().await.clone()
    }

    pub fn set_connectedness(&self, connectedness: Connectedness) {
        self.connectedness.store(connectedness.into(), Ordering::SeqCst);
    }

    pub fn connectedness(&self) -> Connectedness {
        self.connectedness.load(Ordering::SeqCst).into()
    }
}

impl Deref for ArcPeer {
    type Target = Arc<Peer>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Borrow<PeerId> for ArcPeer {
    fn borrow(&self) -> &PeerId {
        &self.id
    }
}

impl PartialEq for ArcPeer {
    fn eq(&self, other: &ArcPeer) -> bool {
        self.id == other.id
    }
}

impl Eq for ArcPeer {}

impl Hash for ArcPeer {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        self.id.hash(hasher)
    }
}

#[derive(Clone)]
pub struct PeerStore {
    peers: Arc<RwLock<HashSet<ArcPeer>>>,
    chain: Arc<RwLock<HashMap<Address, ArcPeer>>>,
}

impl PeerStore {
    pub async fn register(&self, new_peers: Vec<(PublicKey, Multiaddr)>) {
        let mut peers = self.peers.write().await;
        let mut chain = self.chain.write().await;

        for (pubkey, multiaddr) in new_peers.into_iter() {
            let peer_id = pubkey.peer_id();
            let chain_addr = Address::from_pubkey_bytes(pubkey.to_bytes()).expect("chain address from pubkey");
            let peer = ArcPeer::new(peer_id.clone());

            peer.set_pubkey(pubkey).await;
            peer.add_multiaddr(multiaddr).await;
            peers.insert(peer.clone());
            chain.insert(chain_addr, peer);
        }
    }

    pub async fn connectable(&self, max: usize) -> Vec<PeerId> {
        let peers = self.peers.read().await;

        peers.iter().filter_map(|p| {
            if p.connectedness() == Connectedness::CanConnect || p.connectedness() == Connectedness::NotConnected {
                Some(p.id.to_owned())
            } else {
                None
            }
        }).take(max).collect()
    }

    pub async fn peer_ids_by_chain_addr(&self, chain_addrs: Vec<Address>) -> (Vec<PeerId>, Vec<Address>) {
        let chain_book = self.chain.read().await;
        let mut peers = Vec::new();
        let mut not_found = Vec::new();

        for chain_addr in chain_addrs {
            if let Some(peer) = chain_book.get(&chain_addr) {
                peers.push(peer.id.to_owned());
            } else {
                not_found.push(chain_addr);
            }
        }

        (peers, not_found)
    }
}

impl Default for PeerStore {
    fn default() -> Self {
        PeerStore {
            peers: Default::default(),
            chain: Default::default(),
        }
    }
}

#[async_trait]
impl wormhole::peer_store::PeerStore for PeerStore {
    async fn get_pubkey(&self, peer_id: &PeerId) -> Option<PublicKey> {
        if let Some(peer) = self.peers.read().await.get(peer_id).clone() {
            peer.pubkey().await
        } else {
            None
        }
    }

    async fn set_pubkey(&self, peer_id: &PeerId, pubkey: PublicKey) {
        let opt_peer = { self.peers.read().await.get(peer_id).cloned() };

        match opt_peer {
            Some(peer) => peer.set_pubkey(pubkey).await,
            None => {
                let peer = ArcPeer::new(peer_id.to_owned());
                peer.set_pubkey(pubkey).await;
                self.peers.write().await.insert(peer);
            }
        }
    }

    async fn get_connectedness(&self, peer_id: &PeerId) -> Connectedness {
        self.peers.read().await.get(peer_id).map(|p| p.connectedness())
            .unwrap_or_else(|| Connectedness::CannotConnect)
    }

    async fn set_connectedness(&self, peer_id: &PeerId, connectedness: Connectedness) {
        let opt_peer = { self.peers.read().await.get(peer_id).cloned() };

        match opt_peer {
            Some(peer) => peer.set_connectedness(connectedness),
            None => {
                let peer = ArcPeer::new(peer_id.to_owned());
                peer.set_connectedness(connectedness);
                self.peers.write().await.insert(peer);
            }
        }
    }

    async fn get_multiaddrs(&self, peer_id: &PeerId) -> Option<Vec<Multiaddr>> {
        if let Some(peer) = self.peers.read().await.get(peer_id).clone() {
            Some(peer.multiaddrs().await)
        } else {
            None
        }
    }

    async fn add_multiaddr(&self, peer_id: &PeerId, addr: Multiaddr) {
        let opt_peer = { self.peers.read().await.get(peer_id).cloned() };

        match opt_peer {
            Some(peer) => peer.add_multiaddr(addr).await,
            None => {
                let peer = ArcPeer::new(peer_id.to_owned());
                peer.add_multiaddr(addr).await;
                self.peers.write().await.insert(peer);
            }
        }
    }
}
