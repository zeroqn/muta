use anyhow::Error;
use derive_more::Constructor;
use futures::lock::Mutex;
use muta_protocol::types::Address;
use wormhole::{crypto::PeerId, peer_store::PeerInfo};

use std::{
    borrow::Borrow,
    collections::HashSet,
    hash::{Hash, Hasher},
    sync::Arc,
};

#[derive(thiserror::Error, Debug)]
pub enum PeerStoreError {
    #[error("invalid public key")]
    InvalidPublicKey,
}

#[derive(Debug, Constructor)]
struct UserAddress {
    addr:    Address,
    peer_id: PeerId,
}

impl Borrow<Address> for UserAddress {
    fn borrow(&self) -> &Address {
        &self.addr
    }
}

impl PartialEq for UserAddress {
    fn eq(&self, other: &UserAddress) -> bool {
        self.addr == other.addr
    }
}

impl Eq for UserAddress {}

impl Hash for UserAddress {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        self.addr.hash(hasher)
    }
}

#[derive(Clone)]
pub struct PeerStore {
    inner:     wormhole::peer_store::PeerStore,
    user_book: Arc<Mutex<HashSet<UserAddress>>>,
}

impl PeerStore {
    pub fn inner(&self) -> wormhole::peer_store::PeerStore {
        self.inner.clone()
    }

    pub async fn register(&self, peer_info: PeerInfo) -> Result<(), Error> {
        use PeerStoreError::*;

        if let Some(pubkey) = peer_info.pubkey() {
            let addr =
                Address::from_pubkey_bytes(pubkey.to_bytes()).map_err(|_| InvalidPublicKey)?;
            let user_addr = UserAddress::new(addr, peer_info.peer_id().clone());

            self.user_book.lock().await.insert(user_addr);
        }

        self.inner.register(peer_info).await;

        Ok(())
    }

    pub async fn peers_by_user_addrs(
        &self,
        user_addrs: Vec<Address>,
    ) -> (Vec<PeerId>, Vec<Address>) {
        let user_book = self.user_book.lock().await;
        let mut peers = vec![];
        let mut not_found = vec![];

        for user_addr in user_addrs.into_iter() {
            if let Some(peer) = user_book.get(&user_addr).map(|ua| ua.peer_id.clone()) {
                peers.push(peer)
            } else {
                not_found.push(user_addr)
            }
        }

        (peers, not_found)
    }
}

impl Default for PeerStore {
    fn default() -> Self {
        PeerStore {
            inner:     Default::default(),
            user_book: Default::default(),
        }
    }
}
