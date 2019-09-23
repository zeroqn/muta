use std::time::{SystemTime, UNIX_EPOCH};

#[macro_export]
macro_rules! break_ready {
    ($poll:expr) => {
        match $poll {
            Poll::Pending => break,
            Poll::Ready(Some(v)) => v,
            Poll::Ready(None) => return Poll::Ready(()),
        }
    };
}

/// # Panic
pub fn timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("timestamp failure")
        .as_millis()
}
