use futures::channel::mpsc::UnboundedSender;
use log::error;
use tentacle::{
    builder::MetaBuilder,
    bytes::Bytes,
    context::{ProtocolContext, ProtocolContextMutRef},
    service::{ProtocolHandle, ProtocolMeta},
    traits::ServiceProtocol,
    ProtocolId,
};

pub struct MeasureProtocol {
    msg_deliver: UnboundedSender<Bytes>,
}

impl MeasureProtocol {
    pub fn new(msg_deliver: UnboundedSender<Bytes>) -> Self {
        MeasureProtocol { msg_deliver }
    }

    pub fn build_meta(self, proto_id: usize) -> ProtocolMeta {
        let proto_id = ProtocolId::from(proto_id);

        MetaBuilder::new()
            .id(proto_id)
            .service_handle(move || ProtocolHandle::Callback(Box::new(self)))
            .build()
    }
}

impl ServiceProtocol for MeasureProtocol {
    fn init(&mut self, _ctx: &mut ProtocolContext) {
        // Noop
    }

    fn received(&mut self, _ctx: ProtocolContextMutRef, data: Bytes) {
        if self.msg_deliver.unbounded_send(data).is_err() {
            error!("msg deliver dropped");
        }
    }
}
