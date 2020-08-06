mod behaviour;
mod common;
mod identification;
mod message;
mod protocol;

use futures::channel::mpsc::UnboundedSender;
use tentacle::builder::MetaBuilder;
use tentacle::service::{ProtocolHandle, ProtocolMeta};
use tentacle::{ProtocolId, SessionId};

use crate::event::PeerManagerEvent;
use crate::peer_manager::PeerManagerHandle;

use self::protocol::IdentifyProtocol;
use behaviour::IdentifyBehaviour;

pub use self::identification::WaitIdentification;
pub use self::protocol::Error;

pub const NAME: &str = "chain_identify";
pub const SUPPORT_VERSIONS: [&str; 1] = ["0.1"];

#[derive(Clone)]
pub struct Identify {
    proto: IdentifyProtocol,
}

impl Identify {
    pub fn new(peer_mgr: PeerManagerHandle, event_tx: UnboundedSender<PeerManagerEvent>) -> Self {
        #[cfg(feature = "global_ip_only")]
        log::info!("turn on global ip only");
        #[cfg(not(feature = "global_ip_only"))]
        log::info!("turn off global ip only");

        let behaviour = IdentifyBehaviour::new(peer_mgr, event_tx);
        Identify {
            proto: IdentifyProtocol::new(behaviour),
        }
    }

    pub fn build_meta(self, protocol_id: ProtocolId) -> ProtocolMeta {
        MetaBuilder::new()
            .id(protocol_id)
            .name(name!(NAME))
            .support_versions(support_versions!(SUPPORT_VERSIONS))
            .service_handle(move || ProtocolHandle::Callback(Box::new(self.proto)))
            .build()
    }

    pub fn wait_identified(
        &self,
        session_id: SessionId,
    ) -> Result<WaitIdentification, self::protocol::Error> {
        self.proto.wait(session_id)
    }
}
