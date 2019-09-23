use std::{convert::AsRef, fs::File, io::Read, net::SocketAddr, path::Path};

use serde_derive::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub name:          String,
    pub listen:        SocketAddr,
    pub total_packets: isize,
    pub packet_batch:  isize,
    pub core_network:  Option<CoreNetwork>,
    pub tentacle:      Option<Tentacle>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoreNetwork {
    // bootstrap or peer
    pub node:      String,
    pub seckey:    String,
    pub pubkey:    String,
    pub bootstrap: SocketAddr,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Tentacle {
    pub bootstraps: Vec<SocketAddr>,
}

impl Config {
    pub fn parse<P: AsRef<Path>>(conf: P) -> Config {
        let mut buf = Vec::new();
        let mut conf = File::open(conf).expect("open config");

        conf.read_to_end(&mut buf).expect("read config");
        toml::de::from_slice::<Config>(&buf).expect("parse toml")
    }
}