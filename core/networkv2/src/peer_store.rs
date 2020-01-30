use async_trait::async_trait;
use derive_more::Constructor;
use futures::lock::Mutex;
use log::warn;
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
    iter::FromIterator,
    net::IpAddr,
    ops::Deref,
    sync::Arc,
    time::Instant,
};

pub struct PeerInfo {
    id:           PeerId,
    pubkey:       PublicKey,
    net_addrs:    HashSet<Multiaddr>,
    chain_addr:   Address,
    // Protected peer will not be removed unless by force
    is_protected: bool,
    /* TODO: private
     * Private peer will not be shared with non-private peers
     * is_private: bool, */
}

impl PeerInfo {
    pub fn arced(self) -> ArcPeerInfo {
        ArcPeerInfo(Arc::new(self))
    }
}

#[derive(Clone)]
pub struct ArcPeerInfo(Arc<PeerInfo>);

impl ArcPeerInfo {
    pub fn add_net_addr(&self, addr: Multiaddr) -> Self {
        let mut net_addrs = self.net_addrs.clone();
        net_addrs.insert(addr);

        let peer_info = PeerInfo {
            id: self.id.clone(),
            pubkey: self.pubkey.clone(),
            net_addrs,
            chain_addr: self.chain_addr.clone(),
            is_protected: self.is_protected,
        };

        ArcPeerInfo(Arc::new(peer_info))
    }

    pub fn update_net_addrs(&self, addrs: Vec<Multiaddr>) -> Self {
        let peer_info = PeerInfo {
            id:           self.id.clone(),
            pubkey:       self.pubkey.clone(),
            net_addrs:    HashSet::from_iter(addrs),
            chain_addr:   self.chain_addr.clone(),
            is_protected: self.is_protected,
        };

        ArcPeerInfo(Arc::new(peer_info))
    }
}

impl Deref for ArcPeerInfo {
    type Target = PeerInfo;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Borrow<PeerId> for ArcPeerInfo {
    fn borrow(&self) -> &PeerId {
        &self.0.id
    }
}

impl PartialEq for ArcPeerInfo {
    fn eq(&self, other: &ArcPeerInfo) -> bool {
        self.0.id == other.0.id
    }
}

impl Eq for ArcPeerInfo {}

impl Hash for ArcPeerInfo {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        self.0.id.hash(hasher)
    }
}

pub struct PeerInfoByChainAddr(ArcPeerInfo);

impl Deref for PeerInfoByChainAddr {
    type Target = PeerInfo;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Borrow<Address> for PeerInfoByChainAddr {
    fn borrow(&self) -> &Address {
        &self.chain_addr
    }
}

impl PartialEq for PeerInfoByChainAddr {
    fn eq(&self, other: &PeerInfoByChainAddr) -> bool {
        &self.chain_addr == &other.chain_addr
    }
}

impl Eq for PeerInfoByChainAddr {}

impl Hash for PeerInfoByChainAddr {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        self.chain_addr.hash(hasher)
    }
}

impl Into<PeerInfoByChainAddr> for ArcPeerInfo {
    fn into(self) -> PeerInfoByChainAddr {
        PeerInfoByChainAddr(self)
    }
}

pub struct PeerInfoBuilder {
    pubkey:       PublicKey,
    net_addrs:    HashSet<Multiaddr>,
    is_protected: bool,
}

impl PeerInfoBuilder {
    pub fn new(pubkey: PublicKey) -> Self {
        PeerInfoBuilder {
            pubkey,
            net_addrs: HashSet::with_capacity(10),
            is_protected: false,
        }
    }

    pub fn net_addrs(mut self, multiaddrs: Vec<Multiaddr>) -> Self {
        self.net_addrs.extend(multiaddrs);
        self
    }

    pub fn protected(mut self) -> Self {
        self.is_protected = true;
        self
    }

    pub fn build(self) -> PeerInfo {
        let chain_addr = Address::from_pubkey_bytes(self.pubkey.to_bytes())
            .expect("impossible invalid public key");

        PeerInfo {
            id: self.pubkey.peer_id(),
            pubkey: self.pubkey,
            net_addrs: self.net_addrs,
            chain_addr,
            is_protected: self.is_protected,
        }
    }
}

#[derive(Debug, Constructor)]
pub struct NetAddrInfo {
    peer_id:   PeerId,
    multiaddr: Multiaddr,
}

impl Borrow<PeerId> for NetAddrInfo {
    fn borrow(&self) -> &PeerId {
        &self.peer_id
    }
}

impl PartialEq for NetAddrInfo {
    fn eq(&self, other: &NetAddrInfo) -> bool {
        &self.peer_id == &other.peer_id
    }
}

impl Eq for NetAddrInfo {}

impl Hash for NetAddrInfo {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        self.peer_id.hash(hasher)
    }
}

pub struct PeerState {
    connectedness:   Connectedness,
    connected_ip:    Option<IpAddr>,
    retry_count:     u8,
    next_retry:      Instant,
    connected_at:    u64,
    disconnected_at: u64,
    attempt_at:      u64,
    alive_duration:  u64,
}

impl PeerState {
    fn unknown_default() -> Self {
        PeerState {
            connectedness:   Connectedness::NotConnected,
            connected_ip:    None,
            retry_count:     0,
            next_retry:      Instant::now(),
            connected_at:    0,
            disconnected_at: 0,
            attempt_at:      0,
            alive_duration:  0,
        }
    }
}

#[derive(Clone)]
pub struct PeerStore {
    peer_book:    Arc<Mutex<HashSet<ArcPeerInfo>>>,
    chain_book:   Arc<Mutex<HashSet<PeerInfoByChainAddr>>>,
    states:       Arc<Mutex<HashMap<Arc<PeerId>, PeerState>>>,
    unknown_book: Arc<Mutex<HashSet<NetAddrInfo>>>,
}

impl PeerStore {
    pub async fn register_peer(&self, info: PeerInfo) {
        let arced = info.arced();

        {
            self.peer_book.lock().await.insert(arced.clone());
        }
        {
            self.chain_book.lock().await.insert(arced.into());
        }
    }

    pub async fn register_multi_peers(&self, infos: Vec<PeerInfo>) {
        let arced_infos = infos.into_iter().map(PeerInfo::arced).collect::<Vec<_>>();

        {
            self.peer_book.lock().await.extend(arced_infos.clone());
        }
        {
            let mut chain_book = self.chain_book.lock().await;
            chain_book.extend(arced_infos.into_iter().map(|arced| arced.into()));
        }
    }

    pub async fn connectable_peers(&self, number: usize) -> Vec<PeerId> {
        let mut condidates = Vec::with_capacity(number);

        // First try peers we connected before
        {
            let peer_book = self.peer_book.lock().await;
            let states = self.states.lock().await;
            let now = Instant::now();

            let can_connect = |peer_info: &'_ &ArcPeerInfo| -> bool {
                if let Some(state) = states.get(&peer_info.id) {
                    return state.connectedness == Connectedness::CanConnect
                        && now > state.next_retry;
                }

                false
            };

            condidates.extend(
                peer_book
                    .iter()
                    .filter(can_connect)
                    .take(number)
                    .map(|pi| pi.id.to_owned()),
            );

            if condidates.len() == number {
                return condidates;
            }
        }

        let gap = number - condidates.len();

        // Try peers hadn't connected before
        {
            let peer_book = self.peer_book.lock().await;
            let states = self.states.lock().await;

            let not_connected = |peer_info: &'_ &ArcPeerInfo| -> bool {
                if let Some(state) = states.get(&peer_info.id) {
                    return state.connectedness == Connectedness::NotConnected;
                }

                false
            };

            condidates.extend(
                peer_book
                    .iter()
                    .filter(not_connected)
                    .take(gap)
                    .map(|pi| pi.id.to_owned()),
            );

            if condidates.len() == number {
                return condidates;
            }
        }

        let gap = number - condidates.len();

        // Try peers only have peer id and multiaddr
        {
            let unknown_book = self.unknown_book.lock().await;

            condidates.extend(
                unknown_book
                    .iter()
                    .take(gap)
                    .map(|nai| nai.peer_id.to_owned()),
            );
        }

        condidates
    }

    pub async fn peer_ids_by_chain_addrs(
        &self,
        chain_addrs: Vec<Address>,
    ) -> (Vec<PeerId>, Vec<Address>) {
        let mut peer_ids = Vec::with_capacity(chain_addrs.len());
        let mut not_found = vec![];
        let chain_book = self.chain_book.lock().await;

        for chain_addr in chain_addrs.into_iter() {
            if let Some(peer_id) = chain_book.get(&chain_addr).map(|info| info.id.to_owned()) {
                peer_ids.push(peer_id);
            } else {
                not_found.push(chain_addr)
            }
        }

        (peer_ids, not_found)
    }
}

impl Default for PeerStore {
    fn default() -> Self {
        PeerStore {
            peer_book:    Default::default(),
            chain_book:   Default::default(),
            states:       Default::default(),
            unknown_book: Default::default(),
        }
    }
}

#[async_trait]
impl wormhole::peer_store::PeerStore for PeerStore {
    async fn get_pubkey(&self, peer_id: &PeerId) -> Option<PublicKey> {
        let peer_book = self.peer_book.lock().await;

        peer_book.get(peer_id).map(|pi| pi.pubkey.to_owned())
    }

    async fn set_pubkey(&self, peer_id: &PeerId, pubkey: PublicKey) {
        // Only peer ids from unknown book have not pubkey attached
        let opt_nai = { self.unknown_book.lock().await.take(peer_id) };

        if let Some(nai) = opt_nai {
            // NOTE: we don't check whether given pubkey match given peer id
            // because peer id isn't reuse here.
            let peer_info = PeerInfoBuilder::new(pubkey)
                .net_addrs(vec![nai.multiaddr.clone()])
                .build();

            self.register_peer(peer_info).await;
        }
    }

    async fn get_connectedness(&self, peer_id: &PeerId) -> Connectedness {
        let has_pi = { self.peer_book.lock().await.get(peer_id).is_some() };

        if has_pi {
            let states = self.states.lock().await;

            states
                .get(peer_id)
                .map(|st| st.connectedness.to_owned())
                .unwrap_or_else(|| Connectedness::NotConnected)
        } else {
            let has_nai = { self.unknown_book.lock().await.get(peer_id).is_some() };

            if has_nai {
                Connectedness::NotConnected
            } else {
                Connectedness::CannotConnect
            }
        }
    }

    async fn set_connectedness(&self, peer_id: &PeerId, connectedness: Connectedness) {
        let has_pi = { self.peer_book.lock().await.get(peer_id).is_some() };

        if has_pi {
            let mut states = self.states.lock().await;

            let peer_state = states
                .entry(Arc::new(peer_id.to_owned()))
                .or_insert(PeerState::unknown_default());

            peer_state.connectedness = connectedness;
        } else {
            warn!("shouldn't set connectedness on unknown peer {}", peer_id);
        }
    }

    async fn get_multiaddrs(&self, peer_id: &PeerId) -> Option<HashSet<Multiaddr>> {
        let opt_addrs = {
            let peer_book = self.peer_book.lock().await;

            peer_book.get(peer_id).map(|pi| pi.net_addrs.clone())
        };

        if opt_addrs.is_some() {
            opt_addrs
        } else {
            let unknown_book = self.unknown_book.lock().await;

            unknown_book
                .get(peer_id)
                .map(|nai| HashSet::from_iter(vec![nai.multiaddr.to_owned()]))
        }
    }

    async fn add_multiaddr(&self, peer_id: &PeerId, addr: Multiaddr) {
        // Update peer_book
        {
            let mut peer_book = self.peer_book.lock().await;

            if let Some(peer_info) = peer_book.take(peer_id) {
                if !peer_info.net_addrs.contains(&addr) {
                    peer_book.insert(peer_info.add_net_addr(addr));
                } else {
                    peer_book.insert(peer_info);
                }

                return;
            }
        }

        // If not found in peer book then try update unknown book
        let mut unknown_book = self.unknown_book.lock().await;

        if let Some(mut net_addr_info) = unknown_book.take(peer_id) {
            net_addr_info.multiaddr = addr;
            unknown_book.insert(net_addr_info);
        }
    }
}
