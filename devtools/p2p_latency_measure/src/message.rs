use derive_more::Display;
use protocol::types::Hash;
use serde_derive::{Deserialize, Serialize};

use crate::{common::timestamp, payload::Payload};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Display)]
#[display(fmt = "identity: name {}, hash {}", name, hash)]
pub struct Identity {
    name: String,
    hash: String,
}

impl Identity {
    pub fn new(name: String) -> Self {
        let hash = Hash::digest(name.as_bytes().into()).as_hex();

        Identity { name, hash }
    }
}

// Mesaure message has three field:
// 1. identity
// 2. timestamp
// 3. payload (512B/5K/50K/500K)
#[derive(Debug, Serialize, Deserialize, Display)]
#[display(
    fmt = "candy: identity: {}, timestamp: {}, size: {}",
    identity,
    timestamp,
    size
)]
pub struct Candy {
    pub identity:  Identity,
    pub timestamp: u128,
    pub size:      u64,
    pub payload:   Vec<u8>,
}

// Refresh timestamp
impl Clone for Candy {
    fn clone(&self) -> Self {
        Candy {
            identity:  self.identity.clone(),
            timestamp: timestamp(),
            size:      self.size,
            payload:   self.payload.clone(),
        }
    }
}

impl Candy {
    pub fn new(name: String, payload: Payload) -> Self {
        Candy {
            identity:  Identity::new(name),
            timestamp: timestamp(),
            size:      payload.size(),
            payload:   payload.gen(),
        }
    }
}
