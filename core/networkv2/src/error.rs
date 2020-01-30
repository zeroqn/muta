use muta_protocol::{ProtocolError, ProtocolErrorKind};

#[derive(thiserror::Error, Debug)]
pub enum NetworkError {
    #[error("service {0}")]
    Service(#[from] anyhow::Error),
}

impl From<NetworkError> for ProtocolError {
    fn from(err: NetworkError) -> ProtocolError {
        ProtocolError::new(ProtocolErrorKind::Network, Box::new(err))
    }
}
