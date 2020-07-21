use std::sync::Arc;

use derive_more::Display;
use tentacle::context::SessionContext;
use tentacle::error::TransportErrorKind;
use tentacle::multiaddr::Multiaddr;
use tentacle::secio::{PeerId, PublicKey};
use tentacle::{ProtocolId, SessionId};

#[derive(Debug, Display)]
pub enum ProtocolIdentity {
    #[display(fmt = "protocol id {}", _0)]
    Id(ProtocolId),
    #[display(fmt = "protocol name {}", _0)]
    Name(String),
}

#[derive(Debug, Display, PartialEq, Eq)]
pub enum ConnectionType {
    #[display(fmt = "Receive an repeated connection")]
    Inbound,
    #[display(fmt = "Dial an repeated connection")]
    Outbound,
}

#[derive(Debug, Display)]
pub enum ConnectionErrorKind {
    #[display(fmt = "io {:?}", _0)]
    Io(std::io::Error),

    #[display(fmt = "dns resolver {}", _0)]
    DNSResolver(anyhow::Error),

    #[display(fmt = "multiaddr {} is not supported", _0)]
    MultiaddrNotSuppored(Multiaddr),

    #[display(fmt = "handshake {}", _0)]
    SecioHandshake(anyhow::Error),

    #[display(fmt = "remote peer doesn't match one in multiaddr")]
    PeerIdNotMatch,
}

impl ConnectionErrorKind {
    pub fn from_transport_error(err: TransportErrorKind) -> ConnectionErrorKind {
        match err {
            TransportErrorKind::Io(err) => ConnectionErrorKind::Io(err),
            TransportErrorKind::NotSupported(addr) => {
                ConnectionErrorKind::MultiaddrNotSuppored(addr)
            }
            TransportErrorKind::DNSResolverError(_, _) => {
                ConnectionErrorKind::DNSResolver(anyhow::Error::from(err))
            }
        }
    }
}

#[derive(Debug, Display)]
pub enum SessionErrorKind {
    #[display(fmt = "io {:?}", _0)]
    Io(std::io::Error),

    // Maybe unknown protocol, protocol version incompatible, protocol codec
    // error
    #[display(fmt = "{:?} {:?}", identity, cause)]
    Protocol {
        identity: Option<ProtocolIdentity>,
        cause:    Option<anyhow::Error>,
    },

    #[display(fmt = "unexpect {}", _0)]
    Unexpected(anyhow::Error),
}

#[derive(Debug, Display)]
pub enum Event {
    #[display(
        fmt = "new session {} addr {} ty {:?}",
        "ctx.id",
        "ctx.address",
        "ctx.ty"
    )]
    NewSession {
        pubkey: PublicKey,
        ctx:    Arc<SessionContext>,
    },

    #[display(fmt = "connect to {} failed, kind: {}", addr, kind)]
    ConnectFailed {
        addr: Multiaddr,
        kind: ConnectionErrorKind,
    },

    #[display(fmt = "session {} failed, kind: {}", sid, kind)]
    SessionFailed {
        sid:  SessionId,
        kind: SessionErrorKind,
    },

    #[display(fmt = "session {} blocked", "ctx.id")]
    SessionBlocked { ctx: Arc<SessionContext> },

    #[display(fmt = "peer {:?} session {} closed", pid, sid)]
    SessionClosed { pid: PeerId, sid: SessionId },

    #[display(fmt = "repeated connection type {} session {} addr {}", ty, sid, addr)]
    RepeatedConnection {
        ty:   ConnectionType,
        sid:  SessionId,
        addr: Multiaddr,
    },

    #[display(fmt = "bad protocol handle {} cause {}", proto_id, cause)]
    BadProtocolHandle {
        proto_id: ProtocolIdentity,
        cause:    anyhow::Error,
    },

    #[display(fmt = "add listen addr {}", addr)]
    AddNewListenAddr { addr: Multiaddr },

    #[display(fmt = "rmeove listen addr {}", addr)]
    RemoveListenAddr { addr: Multiaddr },

    #[display(fmt = "shutdown")]
    Shutdown,

    #[display(fmt = "connected proto id {} session {}", proto_id, sid)]
    ProtocolConnected {
        proto_id: ProtocolId,
        sid:      SessionId,
    },

    #[display(fmt = "disconnected proto id {} session {}", proto_id, sid)]
    ProtocolDisconnected {
        proto_id: ProtocolId,
        sid:      SessionId,
    },
}
