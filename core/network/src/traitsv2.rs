use std::marker::PhantomData;
use std::pin::Pin;
use std::task::Context;

use arc_swap::ArcSwapOption;
use async_channel::{Sender, TrySendError};
use tentacle::builder::MetaBuilder;
use tentacle::context::{ProtocolContext, ProtocolContextMutRef};
use tentacle::service::ProtocolHandle as TentacleProtocolHandle;
use tentacle::service::ProtocolMeta;
use tentacle::traits::{ServiceProtocol, SessionProtocol};
use tentacle::ProtocolId;

use crate::eventv2::Event;

pub trait Protocol: Send + Sync {
    fn id(&self) -> ProtocolId;
    fn name(&self) -> &'static str;
    fn versions(&self) -> &'static str;
    fn meta(&self) -> ProtocolMeta;
}

pub trait EventSenderExt<E> {
    fn send_event(&self, event: E);
}

impl EventSenderExt<Event> for Sender<Event> {
    #[inline]
    fn send_event(&self, event: Event) {
        match self.try_send(event) {
            Err(TrySendError::Closed(event)) => {
                log::error!("shutdown? failed to send event: {}", event);
            }
            Err(TrySendError::Full(event)) => {
                let self_clone = self.clone();
                tokio::spawn(async move {
                    if let Err(err) = self_clone.send(event).await {
                        log::error!("shutdown? failed to send event: {}", err.into_inner());
                    }
                });
            }
            Ok(()) => (),
        }
    }
}

impl EventSenderExt<Event> for ArcSwapOption<Sender<Event>> {
    #[inline]
    fn send_event(&self, event: Event) {
        let tx_guard = self.load();
        if let Some(event_tx) = tx_guard.as_ref() {
            event_tx.send_event(event);
        }
    }
}
