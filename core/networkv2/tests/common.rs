use wormhole::crypto::{PrivateKey, PublicKey};

#[derive(thiserror::Error, Debug)]
pub enum CommonError {
    #[error("no socket address")]
    NoSocketAddress,

    #[error("no message")]
    NoMessage,
}

pub fn random_keypair() -> (PrivateKey, PublicKey) {
    let privkey = (0..32).map(|_| rand::random::<u8>()).collect::<Vec<_>>();
    let privkey =
        PrivateKey::from_slice(privkey.as_slice()).expect("impossible, random private key fail");

    let pubkey = privkey.pubkey();

    (privkey, pubkey)
}
