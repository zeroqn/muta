use async_channel::Sender;
use tentacle::context::ServiceContext;
use tentacle::error::{DialerErrorKind, ListenErrorKind};
use tentacle::multiaddr::Multiaddr;
use tentacle::service::{ServiceError, ServiceEvent};
use tentacle::traits::ServiceHandle;

use crate::eventv2;
use crate::p2p;
use crate::p2p::event::{ConnectionErrorKind, ConnectionType, ProtocolIdentity, SessionErrorKind};
use crate::traitsv2::EventSenderExt;

pub struct Keeper {
    event_tx: Sender<eventv2::Event>,
}

impl Keeper {
    pub fn new(event_tx: Sender<eventv2::Event>) -> Self {
        Keeper { event_tx }
    }

    #[inline]
    pub fn report(&self, event: eventv2::Event) {
        self.event_tx.send_event(event);
    }

    fn handle_dailer_error(&self, addr: Multiaddr, error: DialerErrorKind) {
        use DialerErrorKind::{
            HandshakeError, IoError, PeerIdNotMatch, RepeatedConnection, TransportError,
        };

        let kind = match error {
            IoError(err) => ConnectionErrorKind::Io(err),
            PeerIdNotMatch => ConnectionErrorKind::PeerIdNotMatch,
            RepeatedConnection(sid) => {
                let ty = ConnectionType::Outbound;
                let repeated_connection = p2p::Event::RepeatedConnection { ty, sid, addr };
                return self.report(eventv2::Event::P2p(repeated_connection));
            }
            HandshakeError(err) => ConnectionErrorKind::SecioHandshake(anyhow::Error::from(err)),
            TransportError(err) => ConnectionErrorKind::from_transport_error(err),
        };

        self.report(eventv2::Event::P2p(p2p::Event::ConnectFailed {
            addr,
            kind,
        }));
    }

    fn handle_listen_error(&self, addr: Multiaddr, error: ListenErrorKind) {
        use ListenErrorKind::{IoError, RepeatedConnection, TransportError};

        let kind = match error {
            IoError(err) => ConnectionErrorKind::Io(err),
            RepeatedConnection(sid) => {
                let ty = ConnectionType::Outbound;
                let repeated_connection = p2p::Event::RepeatedConnection { ty, sid, addr };
                return self.report(eventv2::Event::P2p(repeated_connection));
            }
            TransportError(err) => ConnectionErrorKind::from_transport_error(err),
        };

        self.report(eventv2::Event::P2p(p2p::Event::ConnectFailed {
            addr,
            kind,
        }));
    }
}

impl ServiceHandle for Keeper {
    fn handle_error(&mut self, _: &mut ServiceContext, error: ServiceError) {
        match error {
            ServiceError::DialerError { error, address } => {
                self.handle_dailer_error(address, error)
            }

            ServiceError::ListenError { error, address } => {
                self.handle_listen_error(address, error)
            }

            ServiceError::ProtocolSelectError {
                session_context,
                proto_name,
            } => {
                let protocol_identity = if let Some(proto_name) = proto_name {
                    Some(ProtocolIdentity::Name(proto_name))
                } else {
                    None
                };

                let kind = SessionErrorKind::Protocol {
                    identity: protocol_identity,
                    cause:    None,
                };

                self.report(eventv2::Event::P2p(p2p::Event::SessionFailed {
                    sid: session_context.id,
                    kind,
                }));
            }

            ServiceError::ProtocolError {
                id,
                error,
                proto_id,
            } => {
                let kind = SessionErrorKind::Protocol {
                    identity: Some(ProtocolIdentity::Id(proto_id)),
                    cause:    Some(anyhow::Error::from(error)),
                };

                self.report(eventv2::Event::P2p(p2p::Event::SessionFailed {
                    sid: id,
                    kind,
                }));
            }

            ServiceError::SessionTimeout { session_context } => {
                self.report(eventv2::Event::P2p(p2p::Event::SessionFailed {
                    sid:  session_context.id,
                    kind: SessionErrorKind::Io(std::io::ErrorKind::TimedOut.into()),
                }));
            }

            ServiceError::MuxerError {
                session_context,
                error,
            } => {
                self.report(eventv2::Event::P2p(p2p::Event::SessionFailed {
                    sid:  session_context.id,
                    kind: SessionErrorKind::Io(error),
                }));
            }

            // Bad protocol code, will cause memory leaks/abnormal CPU usage
            ServiceError::ProtocolHandleError { error, proto_id } => {
                self.report(eventv2::Event::P2p(p2p::Event::BadProtocolHandle {
                    proto_id: ProtocolIdentity::Id(proto_id),
                    cause:    anyhow::Error::from(error),
                }));
            }

            // Partial protocol task logic take long time to process, usually
            // indicate bad protocol implement.
            ServiceError::SessionBlocked { session_context } => {
                self.report(eventv2::Event::P2p(p2p::Event::SessionBlocked {
                    ctx: session_context,
                }));
            }
        }
    }

    fn handle_event(&mut self, ctx: &mut ServiceContext, event: ServiceEvent) {
        match event {
            ServiceEvent::SessionOpen { session_context } => {
                let pubkey = match session_context.remote_pubkey.clone() {
                    Some(key) => key,
                    None => {
                        // Peer without encryption will not be able to connect to us
                        log::warn!(
                            "impossible, got a connection from/to {:?} without public key, disconnect it",
                            session_context.address
                        );

                        // Just in case
                        if let Err(e) = ctx.disconnect(session_context.id) {
                            log::error!("disconnect session {} {}", session_context.id, e);
                        }
                        return;
                    }
                };

                self.report(eventv2::Event::P2p(p2p::Event::NewSession {
                    pubkey,
                    ctx: session_context,
                }));
            }
            ServiceEvent::SessionClose { session_context } => {
                let peer_id = match session_context.remote_pubkey.as_ref() {
                    Some(pubkey) => pubkey.peer_id(),
                    None => {
                        log::warn!(
                            "close a session without public key, address {}",
                            session_context.address
                        );
                        return;
                    }
                };

                self.report(eventv2::Event::P2p(p2p::Event::SessionClosed {
                    pid: peer_id,
                    sid: session_context.id,
                }));
            }
            ServiceEvent::ListenStarted { address } => {
                self.report(eventv2::Event::P2p(p2p::Event::AddNewListenAddr {
                    addr: address,
                }));
            }
            ServiceEvent::ListenClose { address } => {
                self.report(eventv2::Event::P2p(p2p::Event::RemoveListenAddr {
                    addr: address,
                }));
            }
        }
    }
}
