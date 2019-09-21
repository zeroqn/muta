use std::time::{SystemTime, UNIX_EPOCH};

/// # Panic
pub fn timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("timestamp failure")
        .as_millis()
}
