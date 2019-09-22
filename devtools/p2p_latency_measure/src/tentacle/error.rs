use std::{error::Error, io};

use derive_more::{Display, From};
use protocol::{ProtocolError, ProtocolErrorKind};

#[derive(Debug, Display, From)]
pub enum ServiceError {
    #[display(fmt = "io error: {}", _0)]
    Io(io::Error),

    #[display(fmt = "tentacle err: {}", _0)]
    Tentacle(tentacle::error::Error),
}

impl Error for ServiceError {}

impl From<ServiceError> for ProtocolError {
    fn from(err: ServiceError) -> ProtocolError {
        ProtocolError::new(ProtocolErrorKind::Network, Box::new(err))
    }
}
