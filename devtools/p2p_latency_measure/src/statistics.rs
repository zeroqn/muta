use std::collections::HashMap;

use derive_more::Display;
use parking_lot::RwLock;

use crate::{message::Identity, payload::Payload};

const INTERVAL_BASE: u128 = 100; // 100ms

#[derive(Debug)]
pub struct IntervalData {
    total: u128,
    data:  HashMap<u128, Vec<u128>>,
}

#[derive(Debug, Display)]
#[display(fmt = "interval: {:?}", data)]
pub struct Interval {
    data: HashMap<String, String>,
}

impl IntervalData {
    pub fn new() -> Self {
        IntervalData {
            total: 0,
            data:  Default::default(),
        }
    }

    pub fn insert(&mut self, latency: u128) {
        self.total += 1;

        let idx = latency / INTERVAL_BASE;

        self.data
            .entry(idx)
            .or_insert_with(Default::default)
            .push(latency)
    }

    pub fn sum(&mut self) -> Interval {
        let mut interval = HashMap::new();

        for (idx, latencies) in self.data.iter() {
            let percent = (latencies.len() as u128 * 100) / self.total;

            let percentage_idx = format!(
                "{}ms - {}ms",
                (idx * INTERVAL_BASE),
                ((idx + 1) * INTERVAL_BASE)
            );
            let percent = format!("{}%", percent);

            interval.insert(percentage_idx, percent);
        }

        Interval { data: interval }
    }
}

#[derive(Debug)]
pub struct Statistics {
    pub data: RwLock<HashMap<Identity, HashMap<Payload, Vec<u128>>>>,
}

#[derive(Debug, Display)]
#[display(fmt = "latency: max {}, min {}, average: {}", max, min, average)]
pub struct Latency {
    pub max:     u128,
    pub min:     u128,
    pub average: u128,
    pub packets: usize,
}

impl Statistics {
    pub fn new() -> Self {
        Statistics {
            data: Default::default(),
        }
    }

    pub fn insert(&self, identity: Identity, payload: Payload, latency: u128) {
        let mut data = self.data.write();

        data.entry(identity)
            .or_insert_with(Default::default)
            .entry(payload)
            .or_insert_with(Default::default)
            .push(latency)
    }

    pub fn average_latencies(&self) -> HashMap<Identity, HashMap<Payload, (Latency, Interval)>> {
        let mut average_latencies = HashMap::new();
        let data = self.data.read();

        for identity in data.keys() {
            let latencies = self.average_identity_latencies(identity);

            average_latencies.insert(identity.clone(), latencies);
        }

        average_latencies
    }

    pub fn average_identity_latencies(
        &self,
        identity: &Identity,
    ) -> HashMap<Payload, (Latency, Interval)> {
        let mut average_latencies = HashMap::new();
        let data = self.data.read();

        if let Some(payload_latencies) = data.get(identity) {
            for (payload, latencies) in payload_latencies.iter() {
                if latencies.is_empty() {
                    continue;
                }

                let max = latencies.iter().max().cloned().expect("no latency data");
                let min = latencies.iter().min().cloned().expect("no latency data");
                let average = latencies.iter().cloned().sum::<u128>() / latencies.len() as u128;
                let packets = latencies.len();

                let latency = Latency {
                    max,
                    min,
                    average,
                    packets,
                };

                // Calculate interval
                let mut interval_data = IntervalData::new();
                for latency in latencies.iter() {
                    interval_data.insert(*latency);
                }

                let interval = interval_data.sum();
                average_latencies.insert(*payload, (latency, interval));
            }
        }

        average_latencies
    }
}
