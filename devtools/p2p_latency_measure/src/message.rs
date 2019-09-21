use std::net::IpAddr;

use derive_more::Display;
use lazy_static::lazy_static;
use protocol::types::Hash;
use rand::random;
use serde_derive::{Deserialize, Serialize};

use crate::{common::timestamp, payload::Payload};

lazy_static! {
    // identity
    pub static ref IDENTITY: String = Hash::digest(random::<u64>().to_be_bytes().as_ref().into()).as_hex();
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Display)]
#[display(fmt = "identity: ip addr {}, hash {}", ip_addr, hash)]
pub struct Identity {
    ip_addr: IpAddr,
    hash:    String,
}

impl Identity {
    pub fn new(ip_addr: IpAddr) -> Self {
        Identity {
            ip_addr,
            hash: IDENTITY.to_owned(),
        }
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
    pub fn new(ip_addr: IpAddr, payload: Payload) -> Self {
        Candy {
            identity:  Identity::new(ip_addr),
            timestamp: timestamp(),
            size:      payload.size(),
            payload:   payload.gen(),
        }
    }
}
