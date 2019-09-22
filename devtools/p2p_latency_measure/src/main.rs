mod common;
mod latency_measure;
mod message;
mod payload;
mod statistics;

use latency_measure::{MeasureLatency, END_GOSSIP_TEST_PAYLOAD};
use statistics::Statistics;

use std::{
    env,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    thread,
    time::Duration,
};

use core_network::{NetworkConfig, NetworkService};
use lazy_static::lazy_static;
use log::info;
use tentacle_secio::SecioKeyPair;

const MEASURE_GOSSIP_TIMES: isize = 1000;

lazy_static! {
    pub static ref BOOTSTRAP_SECKEY: String = "8".repeat(32);
}

#[derive(Debug)]
struct Config {
    pub bootstrap:         Option<SocketAddr>,
    pub listen:            SocketAddr,
    pub public_ip:         IpAddr,
    pub total_packets:     isize,
    pub packet_batch_size: isize,
}

impl Config {
    pub fn parse() -> Self {
        let mut args = env::args();

        let bootstrap = args.nth(1).expect("bootstrap").parse::<SocketAddr>().ok();

        let listen = args
            .nth(0)
            .expect("listen address")
            .parse::<SocketAddr>()
            .expect("socket address");

        let public_ip = args
            .nth(0)
            .expect("our public ip")
            .parse::<IpAddr>()
            .expect("ip address");

        let packet_batch_size = args
            .nth(0)
            .expect("packet batch size")
            .parse::<isize>()
            .expect("packet batch isize");

        let total_packets = args
            .nth(0)
            .expect("total packet")
            .parse::<isize>()
            .unwrap_or(MEASURE_GOSSIP_TIMES);

        Config {
            bootstrap,
            listen,
            public_ip,
            total_packets,
            packet_batch_size,
        }
    }
}

#[runtime::main(runtime_tokio::Tokio)]
async fn main() {
    env_logger::init();

    // Bootstrap keys
    let bt_keypair =
        SecioKeyPair::secp256k1_raw_key(BOOTSTRAP_SECKEY.as_str()).expect("bootstrap seckey");
    let bt_pubkey = bt_keypair.to_public_key();

    // Initialize statistics table
    let statistics = Arc::new(Statistics::new());

    // Parse args
    let config = Config::parse();

    let mut node = if let Some(bootstrap) = config.bootstrap {
        info!("Peer");

        let peer_conf = NetworkConfig::new()
            .bootstraps(vec![(bt_pubkey.encode(), bootstrap)])
            .expect("bootstrap failure");

        let mut peer = NetworkService::new(peer_conf);
        peer.listen(config.listen).expect("listen failure");

        peer
    } else {
        info!("Bootstrap");

        // Configure bootstrap
        let bt_conf = NetworkConfig::new().skp(bt_keypair);

        let mut bootstrap = NetworkService::new(bt_conf);
        bootstrap.listen(config.listen).expect("listen failure");

        bootstrap
    };

    // Register measure latency
    let our_ip = Arc::new(config.public_ip);
    let total_packets = Arc::new(config.total_packets);
    let packet_batch = Arc::new(config.packet_batch_size);
    let handle = Arc::new(node.handle());
    let measure_latency =
        MeasureLatency::new(our_ip, total_packets, packet_batch, handle, statistics);

    node.register_endpoint_handler(END_GOSSIP_TEST_PAYLOAD, Box::new(measure_latency.clone()))
        .expect("register failure");

    // Start node
    runtime::spawn(node);

    // Sleep a while for discovery
    thread::sleep(Duration::from_secs(10));

    // Start latency measurement
    measure_latency.start();

    // Print statistics
    info!(
        "statistics: {:?}",
        measure_latency.statistics().average_latencies()
    );
}
