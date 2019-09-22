use std::{net::IpAddr, sync::Arc, thread, time::Duration};

use async_trait::async_trait;
use derive_more::Constructor;
use log::info;
use protocol::{
    traits::{Context, Gossip, MessageHandler, Priority},
    ProtocolResult,
};

use crate::{common::timestamp, message::Candy, payload::Payload, statistics::Statistics};

pub const END_GOSSIP_TEST_PAYLOAD: &str = "/gossip/benchmark/measure_latency";

#[derive(Constructor, Clone)]
pub struct MeasureLatency<G: Gossip + 'static> {
    pub me:            Arc<IpAddr>,
    pub total_packets: Arc<isize>,
    pub packet_batch:  Arc<isize>,
    pub gossip:        Arc<G>,
    pub statistics:    Arc<Statistics>,
}

impl<G: Gossip + 'static> MeasureLatency<G> {
    pub fn start(&self) {
        info!("Starting measure latency");
        info!("Loop times: {}", self.total_packets);

        for payload in Payload::iter() {
            info!("Using payload size {}", payload);

            let ip_addr = *self.me.as_ref();
            let candy = Candy::new(ip_addr, *payload);
            let gossip = Arc::clone(&self.gossip);
            let mut gossip_countdown = *self.total_packets.as_ref();

            while gossip_countdown > 0 {
                if gossip_countdown % *self.packet_batch.as_ref() == 0 {
                    thread::sleep(Duration::from_secs(5));
                }

                let gossip = Arc::clone(&gossip);
                let candy = candy.clone();

                runtime::spawn(async move {
                    let _ = gossip
                        .broadcast(
                            Context::new(),
                            END_GOSSIP_TEST_PAYLOAD,
                            candy,
                            Priority::High,
                        )
                        .await;
                });

                gossip_countdown -= 1;
            }

            info!("End payload size {}", payload);
            info!("Sleep 10s");
            thread::sleep(Duration::from_secs(10));
        }
    }

    pub fn statistics(&self) -> Arc<Statistics> {
        Arc::clone(&self.statistics)
    }
}

#[async_trait]
impl<G: Gossip + 'static> MessageHandler for MeasureLatency<G> {
    type Message = Candy;

    async fn process(&self, _ctx: Context, msg: Self::Message) -> ProtocolResult<()> {
        let payload = Payload::from(msg.size);
        let latency = timestamp() - msg.timestamp;

        self.statistics
            .insert(msg.identity.clone(), payload, latency);

        Ok(())
    }
}
