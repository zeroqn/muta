use muta_protocol::{ProtocolError, ProtocolErrorKind};

#[derive(thiserror::Error, Debug)]
pub enum NetworkError {
    #[error("unsupported endpoint {0}")]
    UnsupportedEndpoint(&'static str),

    #[error("service {0}")]
    Service(#[from] anyhow::Error),

    #[error("listen {0}")]
    Listen(anyhow::Error),
}

impl From<NetworkError> for ProtocolError {
    fn from(err: NetworkError) -> ProtocolError {
        ProtocolError::new(ProtocolErrorKind::Network, Box::new(err))
    }
}
