use std::{convert::AsRef, fs::File, io::Read, net::SocketAddr, path::Path};

use serde_derive::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub name:          String,
    pub listen:        SocketAddr,
    pub total_packets: isize,
    pub packet_batch:  isize,
    pub core_network:  Option<CoreNetwork>,
    pub tentacle:      Option<Tentacle>,
    pub tokio:         Option<TokioNet>,
}

#[derive(Debug, Deserialize)]
pub struct Bootstrap {
    // bootstrap or peer
    pub pubkey: String,
    pub addr:   SocketAddr,
}

#[derive(Debug, Deserialize)]
pub struct CoreNetwork {
    pub ty:         String,
    pub seckey:     String,
    pub bootstraps: Vec<Bootstrap>,
}

#[derive(Debug, Deserialize)]
pub struct Tentacle {
    pub bootstraps: Vec<SocketAddr>,
}

#[derive(Debug, Deserialize)]
pub struct TokioNet {
    pub nodes: Vec<SocketAddr>,
}

impl Config {
    pub fn parse<P: AsRef<Path>>(conf: P) -> Config {
        let mut buf = Vec::new();
        let mut conf = File::open(conf).expect("open config");

        conf.read_to_end(&mut buf).expect("read config");
        toml::de::from_slice::<Config>(&buf).expect("parse toml")
    }
}
