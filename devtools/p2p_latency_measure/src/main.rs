mod common;
mod gossip;
mod message;
mod payload;
mod statistics;

use gossip::{MeasureLatency, END_GOSSIP_TEST_PAYLOAD};
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
    let mut args = env::args();

    let bootstrap = args.nth(1).expect("bootstrap");
    let listen = args
        .nth(0)
        .expect("listen address")
        .parse::<SocketAddr>()
        .expect("socket address");
    let our_ip = args
        .nth(0)
        .expect("our public ip")
        .parse::<IpAddr>()
        .expect("ip address");
    let packet_batch = args
        .nth(0)
        .expect("packet batch size")
        .parse::<isize>()
        .expect("packet batch isize");

    let (mut node, handle) = if bootstrap == "bootstrap" {
        info!("Bootstrap");

        // Configure bootstrap
        let bt_conf = NetworkConfig::new().skp(bt_keypair);

        let mut bootstrap = NetworkService::new(bt_conf);
        let handle = bootstrap.handle();
        bootstrap.listen(listen).expect("listen failure");

        (bootstrap, handle)
    } else {
        info!("Peer");

        let bt_addr = bootstrap.parse::<SocketAddr>().expect("socket address");

        let peer_conf = NetworkConfig::new()
            .bootstraps(vec![(bt_pubkey.encode(), bt_addr)])
            .expect("bootstrap failure");

        let mut peer = NetworkService::new(peer_conf);
        let handle = peer.handle();
        peer.listen(listen).expect("listen failure");

        (peer, handle)
    };

    // Register measure latency
    let our_ip = Arc::new(our_ip);
    let packet_batch = Arc::new(packet_batch);
    let measure_latency = MeasureLatency::new(our_ip, packet_batch, Arc::new(handle), statistics);

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
