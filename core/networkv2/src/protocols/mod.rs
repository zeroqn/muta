mod rpc;
pub use rpc::RpcService;

use lazy_static::lazy_static;

use std::sync::atomic::{AtomicU64, Ordering};

lazy_static! {
    pub static ref NEXT_PROTOCOL_ID: AtomicU64 = AtomicU64::new(1);
}

fn next_protocol_id() -> u64 {
    NEXT_PROTOCOL_ID.fetch_add(1, Ordering::SeqCst)
}
