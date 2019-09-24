// node::build(&config)
// node.listen()
// node.handle() for gossip
// node.register() measure_latency
// node imple Future
mod gossip;

use gossip::TokioGossip;

use std::{
    future::Future,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{
    compat::{Compat01As03, Stream01CompatExt},
    pin_mut,
};
use protocol::{
    traits::{MessageCodec, MessageHandler},
    ProtocolResult,
};
use tokio::net::{tcp::Incoming, TcpListener, TcpStream};

use crate::config::Config;

pub struct TokioNode {
    incoming: Option<Compat01As03<Incoming>>,
}

impl TokioNode {
    pub fn build(config: &Config) -> Self {
        unimplemented!()
    }

    pub fn handle(&mut self) -> TokioGossip {
        unimplemented!()
    }

    pub fn listen(&mut self, socket_addr: SocketAddr) {
        let listener = TcpListener::bind(&socket_addr).expect("listen");
        self.incoming = Some(listener.incoming().compat());
    }

    pub fn register<M>(
        &mut self,
        _: &str,
        handler: impl MessageHandler<Message = M>,
    ) -> ProtocolResult<()>
    where
        M: MessageCodec + Unpin,
    {
        unimplemented!()
    }
}

impl Future for TokioNode {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {
        loop {
            if let Some(incoming) = &self.as_mut().incoming {
                pin_mut!(incoming);

                let (TcpStream, SocketAddr) = crate::break_ready!(incoming);
            }
        }

        Poll::Pending
    }
}
