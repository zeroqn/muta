#[macro_use]
mod common;
mod ccore_network;
mod config;
mod latency_measure;
mod message;
mod payload;
mod statistics;
mod tentacle;
mod tokio_net;

use self::tentacle::TentacleNode;
use ccore_network::CoreNetworkNode;
use config::Config;
use latency_measure::{MeasureLatency, END_GOSSIP_TEST_PAYLOAD};
use statistics::Statistics;
use tokio_net::TokioNode;

use std::{env, net::SocketAddr, str::FromStr, sync::Arc, thread, time::Duration};

use log::info;

enum Target {
    Tentacle,
    CoreNetwork,
    Tokio,
}

impl FromStr for Target {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let tar = match s {
            "tentacle" => Target::Tentacle,
            "core_network" => Target::CoreNetwork,
            "tokio" => Target::Tokio,
            _ => return Err(()),
        };

        Ok(tar)
    }
}

macro_rules! start_node {
    ($node:ident, $args:expr) => {{
        // Initialize statistics table
        let statistics = Arc::new(Statistics::new());

        // Parse config
        let config_file = $args.nth(0).expect("config file");
        let config = Config::parse(config_file);

        let mut node = $node::build(&config);

        // Parse listen
        let listen = $args
            .nth(0)
            .map(|listen| listen.parse::<SocketAddr>().expect("listen"))
            .unwrap_or_else(|| config.listen);

        node.listen(listen);

        // Parse name
        let name = Arc::new($args.nth(0).unwrap_or(config.name));

        // Register measure latency
        let total_packets = Arc::new(config.total_packets);
        let packet_batch = Arc::new(config.packet_batch);
        let handle = Arc::new(node.handle());

        let measure_latency =
            MeasureLatency::new(name, total_packets, packet_batch, handle, statistics);

        node.register(END_GOSSIP_TEST_PAYLOAD, measure_latency.clone())
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
    }};
}

// cargo run --relase [target] [config] [listen] [name]
#[runtime::main(runtime_tokio::Tokio)]
async fn main() {
    // Enable Info log by default
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info");
    }

    env_logger::init();

    let mut args = env::args();

    // Parse target
    let target = Target::from_str(&args.nth(1).expect("target")).expect("target");

    // Start measure
    match target {
        Target::Tentacle => start_node!(TentacleNode, args),
        Target::CoreNetwork => start_node!(CoreNetworkNode, args),
        Target::Tokio => start_node!(TokioNode, args),
    }
}
