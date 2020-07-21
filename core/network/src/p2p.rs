pub mod crypto;
mod event;
pub mod keeper;

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use arc_swap::ArcSwapOption;
use async_channel::Sender;
use dashmap::DashMap;
use futures::stream::Stream;
use protocol::traits::Priority;
use protocol::Bytes;
use secrecy::{ExposeSecret, Secret};
use tentacle::builder::ServiceBuilder;
use tentacle::error::SendErrorKind;
use tentacle::multiaddr::Multiaddr;
use tentacle::service::{Service, ServiceControl, TargetProtocol, TargetSession};
use tentacle::{ProtocolId, SessionId};

use crypto::KeyPair;
use keeper::Keeper;

use crate::eventv2;
use crate::traitsv2::{EventSenderExt, Protocol};

pub use self::event::Event;

type ProtocolName = &'static str;
type ProtocolMap = Arc<DashMap<ProtocolName, Box<dyn Protocol>>>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no listen yet")]
    NoListen,

    #[error("shutdown")]
    Shutdown,

    #[error("busy")]
    Busy,
}

impl From<SendErrorKind> for Error {
    fn from(err: SendErrorKind) -> Error {
        match err {
            SendErrorKind::BrokenPipe => Error::Shutdown,
            SendErrorKind::WouldBlock => Error::Busy,
        }
    }
}

// #[derive(Debug, Display)]
// #[display(fmt = "protocol replaced {}", _0)]
// pub struct ProtocolReplaced(ProtocolName);

#[derive(Debug, thiserror::Error)]
#[error("protocol replaced {0}")]
pub struct ProtocolReplaced(ProtocolName);

pub struct Config {
    /// Secio keypair for connection encryption and peer identity
    pub keypair: Secret<KeyPair>,

    /// Max stream window size
    pub max_frame_length: Option<usize>,

    /// Send buffer size
    pub send_buffer_size: Option<usize>,

    /// Write buffer size
    pub recv_buffer_size: Option<usize>,

    /// Max wait streams
    pub max_wait_streams: Option<usize>,

    /// Write timeout
    pub write_timeout: Option<u64>,
}

pub struct Message {
    recipients:  TargetSession,
    protocol_id: ProtocolId,
    content:     Bytes,
}

impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("P2P Message")
            .field("recipients", &self.recipients)
            .field("protocol_id", &self.protocol_id.value())
            .field("message_length", &self.content.len())
            .finish()
    }
}

impl Message {
    pub fn new(recipients: TargetSession, protocol_id: ProtocolId, content: Bytes) -> Self {
        Message {
            recipients,
            protocol_id,
            content,
        }
    }
}

#[derive(Clone)]
pub struct Handle {
    control: ServiceControl,
}

impl Handle {
    pub fn new(control: ServiceControl) -> Self {
        Handle { control }
    }
}

#[derive(Clone)]
pub struct P2p {
    config: Arc<Config>,

    handle:    ArcSwapOption<Handle>,
    protocols: Arc<DashMap<ProtocolName, ProtocolId>>,
    event_tx:  Sender<eventv2::Event>,

    _protocols: Arc<DashMap<ProtocolName, Box<dyn Protocol>>>,
}

impl P2p {
    pub fn new(event_tx: Sender<eventv2::Event>, config: Config) -> Self {
        P2p {
            config: Arc::new(config),

            handle: ArcSwapOption::from(None),
            protocols: Default::default(),
            event_tx,

            _protocols: Default::default(),
        }
    }

    pub fn dial(&self, multiaddr: Multiaddr) -> Result<(), Error> {
        self.with_handle(|Handle { control }| -> _ {
            control.dial(multiaddr, TargetProtocol::All)?;

            Ok(())
        })
    }

    pub fn disconnect(&self, session_id: SessionId) -> Result<(), Error> {
        self.with_handle(|Handle { control }| -> _ {
            control.disconnect(session_id)?;

            Ok(())
        })
    }

    pub fn register_protocol(&self, protocol: impl Protocol + 'static) -> Option<ProtocolReplaced> {
        let protocol_name = protocol.name();

        self.protocols.insert(protocol_name, protocol.id());
        if self._protocols.insert(protocol_name, Box::new(protocol)) {
            Some(ProtocolReplaced(protocol_name))
        } else {
            None
        }
    }

    #[inline]
    pub fn send_message(&self, message: Message, priority: Priority) -> Result<(), Error> {
        self.with_handle(|Handle { control }| -> _ {
            let Message {
                recipients,
                protocol_id,
                content,
            } = message;

            match priority {
                Priority::High => {
                    control.quick_filter_broadcast(recipients, protocol_id, content)?
                }
                Priority::Normal => control.filter_broadcast(recipients, protocol_id, content)?,
            }

            Ok(())
        })
    }

    pub async fn listen(&self, multiaddr: Multiaddr) -> Result<(), anyhow::Error> {
        let keeper = Keeper::new(self.event_tx.clone());
        let mut background_service = BackgroundService::new(
            &self.config,
            Arc::clone(&self._protocols),
            keeper,
            self.event_tx.clone(),
        );
        background_service.listen(multiaddr).await?;

        let handle = Handle::new(background_service.control());
        self.handle.store(Some(Arc::new(handle)));

        tokio::spawn(background_service);

        Ok(())
    }

    #[inline]
    fn with_handle<F>(&self, f: F) -> Result<(), Error>
    where
        F: FnOnce(&Handle) -> Result<(), Error>,
    {
        let handle_guard = self.handle.load();
        let handle = handle_guard.as_ref().ok_or_else(|| Error::NoListen)?;

        f(handle.as_ref())
    }
}

// BackgroundService is a wrapper around tentacle `Service`.
struct BackgroundService {
    inner:    Service<Keeper>,
    event_tx: Sender<eventv2::Event>,
}

impl BackgroundService {
    fn new(
        config: &Config,
        protocol_map: ProtocolMap,
        keeper: Keeper,
        tx: Sender<eventv2::Event>,
    ) -> Self {
        let mut builder = ServiceBuilder::default()
            .key_pair(config.keypair.expose_secret().inner().to_owned())
            .forever(true);

        let mut yamux_config = tentacle::yamux::Config::default();

        if let Some(max) = config.max_wait_streams {
            yamux_config.accept_backlog = max;
        }

        if let Some(timeout) = config.write_timeout {
            yamux_config.connection_write_timeout = Duration::from_secs(timeout);
        }

        builder = builder.yamux_config(yamux_config);

        if let Some(max) = config.max_frame_length {
            builder = builder.max_frame_length(max);
        }

        if let Some(size) = config.send_buffer_size {
            builder = builder.set_send_buffer_size(size);
        }

        if let Some(size) = config.recv_buffer_size {
            builder = builder.set_recv_buffer_size(size);
        }

        for elem_guard in protocol_map.iter() {
            let (name, protocol) = elem_guard.pair();
            log::debug!("insert protocol {}", name);

            builder = builder.insert_protocol(protocol.meta());
        }

        BackgroundService {
            inner:    builder.build(keeper),
            event_tx: tx,
        }
    }

    fn control(&self) -> ServiceControl {
        self.inner.control().to_owned()
    }

    async fn listen(&mut self, multiaddr: Multiaddr) -> Result<(), anyhow::Error> {
        self.inner.listen(multiaddr).await?;

        Ok(())
    }
}

impl Future for BackgroundService {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match Stream::poll_next(Pin::new(&mut self.as_mut().inner), ctx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(())) => (),
                Poll::Ready(None) => {
                    self.event_tx
                        .send_event(eventv2::Event::P2p(self::Event::Shutdown));
                    return Poll::Ready(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, KeyPair, Message, P2p};

    use std::sync::Arc;
    use std::time::Duration;

    use arc_swap::ArcSwapOption;
    use async_channel::{Receiver, Sender};
    use once_cell::sync::Lazy;
    use protocol::traits::Priority;
    use protocol::types::Hash;
    use protocol::Bytes;
    use secrecy::Secret;
    use tentacle::builder::MetaBuilder;
    use tentacle::context::ProtocolContextMutRef;
    use tentacle::multiaddr::Multiaddr;
    use tentacle::service::{ProtocolHandle, ProtocolMeta, TargetSession};
    use tentacle::traits::SessionProtocol;
    use tentacle::ProtocolId;

    use crate::commonv2;
    use crate::eventv2;
    use crate::p2p;
    use crate::traitsv2::{EventSenderExt, Protocol};

    const ECHO_PROTOCOL: &str = "echo";
    const ECHO_PROTOCOL_ID: usize = 1usize;
    const ECHO_PASSWORD: &'static [u8] = b"dan xing hao shi";
    const ECHO_BACK: &'static [u8] = b"mo wen qian cheng";
    const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);

    static ECHO_SENDER: Lazy<ArcSwapOption<Sender<&'static [u8]>>> =
        Lazy::new(|| ArcSwapOption::new(None));

    struct EchoProtocol {
        event_tx: Sender<eventv2::Event>,
    }

    impl EchoProtocol {
        fn new(event_tx: Sender<eventv2::Event>) -> Self {
            EchoProtocol { event_tx }
        }
    }

    impl Protocol for EchoProtocol {
        fn id(&self) -> ProtocolId {
            ECHO_PROTOCOL_ID.into()
        }

        fn name(&self) -> &'static str {
            ECHO_PROTOCOL
        }

        fn versions(&self) -> &'static str {
            "0.1"
        }

        fn meta(&self) -> ProtocolMeta {
            let event_tx = self.event_tx.clone();
            let name = self.name();

            MetaBuilder::new()
                .id(self.id())
                .name(move |_| name.to_owned())
                .support_versions(vec![self.versions().to_owned()])
                .session_handle(move || {
                    ProtocolHandle::Callback(Box::new(EchoProtocol {
                        event_tx: event_tx.clone(),
                    }))
                })
                .build()
        }
    }

    impl SessionProtocol for EchoProtocol {
        fn connected(&mut self, context: ProtocolContextMutRef, _version: &str) {
            self.event_tx
                .send_event(eventv2::Event::P2p(p2p::Event::ProtocolConnected {
                    proto_id: context.proto_id(),
                    sid:      context.session.id,
                }))
        }

        fn received(&mut self, context: ProtocolContextMutRef, data: Bytes) {
            match data.as_ref() {
                ECHO_PASSWORD => context
                    .quick_send_message(Bytes::copy_from_slice(ECHO_BACK))
                    .expect("echo"),
                ECHO_BACK => {
                    let guard = ECHO_SENDER.load();
                    let echo_tx = guard.as_ref().expect("echo_tx");
                    echo_tx.try_send(ECHO_BACK).expect("echo back");
                }
                _ => (),
            }
        }
    }

    fn create_node<S: AsRef<[u8]>>(
        seckey: S,
    ) -> Result<(P2p, Receiver<eventv2::Event>), anyhow::Error> {
        let (event_tx, event_rx) = async_channel::bounded(900);
        let config = Config {
            keypair:          Secret::new(KeyPair::from_slice(seckey)?),
            max_frame_length: None,
            send_buffer_size: None,
            recv_buffer_size: None,
            max_wait_streams: None,
            write_timeout:    None,
        };

        let p2p = P2p::new(event_tx.clone(), config);
        assert!(p2p
            .register_protocol(EchoProtocol::new(event_tx.clone()))
            .is_none());

        Ok((p2p, event_rx))
    }

    async fn wait_event<F: Fn(&p2p::Event) -> bool>(
        event_rx: &Receiver<eventv2::Event>,
        predicate: F,
    ) -> Result<p2p::Event, anyhow::Error> {
        loop {
            match event_rx.recv().await? {
                eventv2::Event::P2p(event) if predicate(&event) => return Ok(event),
                _ => continue,
            }
        }
    }

    #[tokio::test(threaded_scheduler)]
    async fn should_basic_work() -> Result<(), anyhow::Error> {
        let (libing, libing_pigeon) =
            create_node(Hash::digest(Bytes::copy_from_slice(b"libing")).as_bytes())?;
        let (cuibei, cuibei_pigeon) =
            create_node(Hash::digest(Bytes::copy_from_slice(b"cuibei")).as_bytes())?;

        let libing_addr: Multiaddr = "/ip4/127.0.0.1/tcp/3000".parse()?;
        let cuibei_addr: Multiaddr = "/ip4/127.0.0.1/tcp/3001".parse()?;

        libing.listen(libing_addr.clone()).await?;
        cuibei.listen(cuibei_addr).await?;

        // Make sure both are ready
        commonv2::timeout(
            DEFAULT_TIMEOUT,
            wait_event(&libing_pigeon, |event| match event {
                p2p::Event::AddNewListenAddr { .. } => true,
                _ => false,
            }),
        )
        .await?;

        commonv2::timeout(
            DEFAULT_TIMEOUT,
            wait_event(&cuibei_pigeon, |event| match event {
                p2p::Event::AddNewListenAddr { .. } => true,
                _ => false,
            }),
        )
        .await?;

        // Don't need libing pigeon anymore
        tokio::spawn(async move {
            loop {
                let _ = libing_pigeon.recv().await;
            }
        });
        cuibei.dial(libing_addr)?;

        commonv2::timeout(
            DEFAULT_TIMEOUT,
            wait_event(&cuibei_pigeon, |event| match event {
                p2p::Event::NewSession { .. } => true,
                _ => false,
            }),
        )
        .await?;

        // Protocol also opened, we are ready now.
        commonv2::timeout(
            DEFAULT_TIMEOUT,
            wait_event(&cuibei_pigeon, |event| match event {
                p2p::Event::ProtocolConnected { .. } => true,
                _ => false,
            }),
        )
        .await?;

        let (echo_tx, echo_rx) = async_channel::bounded(10);
        ECHO_SENDER.store(Some(Arc::new(echo_tx)));

        let message = Message::new(
            TargetSession::All,
            ECHO_PROTOCOL_ID.into(),
            Bytes::from_static(ECHO_PASSWORD),
        );
        cuibei.send_message(message, Priority::High)?;

        let echo_back = commonv2::timeout(DEFAULT_TIMEOUT, echo_rx.recv()).await?;
        assert_eq!(echo_back, ECHO_BACK);

        cuibei.disconnect(1.into())?;

        commonv2::timeout(
            DEFAULT_TIMEOUT,
            wait_event(&cuibei_pigeon, |event| match event {
                p2p::Event::SessionClosed { .. } => true,
                _ => false,
            }),
        )
        .await?;

        Ok(())
    }
}
