use derive_more::Display;

use std::{convert::TryFrom, ops::Deref};

pub const MULTI_CAST_SCHEME: &str = "/muta/multicast";
pub const RPC_SCHEME: &str = "/muta/rpc";

pub const MAX_ENDPOINT_LENGTH: usize = 120;

#[derive(thiserror::Error, Debug)]
pub enum EndpointError {
    #[error("too long")]
    TooLong,

    #[error("unsupported scheme {0}")]
    UnsupportedScheme(&'static str),

    #[error("invalid {0}")]
    Invalid(&'static str),
}

#[derive(Debug, Display, PartialEq, Eq)]
pub enum EndpointScheme {
    #[display(fmt = "{}", MULTI_CAST_SCHEME)]
    MultiCast,

    #[display(fmt = "{}", RPC_SCHEME)]
    Rpc,
}

#[derive(Debug, Clone, Display)]
#[display(fmt = "{}", _0)]
pub struct Endpoint(&'static str);

impl Endpoint {
    pub fn scheme(&self) -> EndpointScheme {
        if self.starts_with(MULTI_CAST_SCHEME) {
            EndpointScheme::MultiCast
        } else if self.starts_with(RPC_SCHEME) {
            EndpointScheme::Rpc
        } else {
            unreachable!()
        }
    }
}

impl TryFrom<&'static str> for Endpoint {
    type Error = EndpointError;

    fn try_from(endpoint: &'static str) -> Result<Self, Self::Error> {
        use EndpointError::*;

        if endpoint.is_empty() {
            return Err(Invalid(endpoint));
        }

        if endpoint.len() > MAX_ENDPOINT_LENGTH {
            return Err(TooLong);
        }

        if !endpoint.starts_with(MULTI_CAST_SCHEME) && !endpoint.starts_with(RPC_SCHEME) {
            return Err(UnsupportedScheme(endpoint));
        }
    }
}

impl Deref for Endpoint {
    type Target = str;

    fn deref(&self) -> &'static str {
        self.0
    }
}
