use secrecy::Zeroize;
use tentacle::secio::SecioKeyPair;

use core::{ptr, sync::atomic};

pub const SECRET_KEY_SIZE: usize = 32;

#[derive(Clone)]
pub struct KeyPair(SecioKeyPair);

impl KeyPair {
    pub fn new(keypair: SecioKeyPair) -> Self {
        KeyPair(keypair)
    }

    pub fn from_slice<S: AsRef<[u8]>>(seckey: S) -> Result<Self, anyhow::Error> {
        let keypair = SecioKeyPair::secp256k1_raw_key(seckey)?;

        Ok(KeyPair(keypair))
    }

    pub fn inner(&self) -> &SecioKeyPair {
        &self.0
    }
}

impl Default for KeyPair {
    fn default() -> KeyPair {
        let keypair = SecioKeyPair::secp256k1_raw_key(&[0u8; SECRET_KEY_SIZE])
            .expect("wrong secret key size");
        KeyPair(keypair)
    }
}

// TODO: verify
// FIXME:
impl Zeroize for KeyPair {
    fn zeroize(&mut self) {
        // unsafe { ptr::write_volatile(self, Self::default()) }
        // atomic::compiler_fence(atomic::Ordering::SeqCst);
    }
}
