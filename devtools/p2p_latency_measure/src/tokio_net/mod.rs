mod gossip;
mod recorder;

use gossip::TokioGossip;
use recorder::Recorder;

use std::{
    collections::HashSet,
    future::Future,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{
    channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender},
    compat::{Compat01As03, Stream01CompatExt},
    pin_mut,
    stream::Stream,
};
use log::{error, info};
use protocol::{
    traits::{MessageCodec, MessageHandler},
    ProtocolResult,
};
use tokio::net::{tcp::Incoming, TcpListener, TcpStream};

use crate::config::Config;

struct Inner {
    incoming:  Compat01As03<Incoming>,
    stream_tx: UnboundedSender<TcpStream>,

    // Save ip addr, make sure no duplicate ip
    in_addrs: HashSet<IpAddr>,
}

impl Inner {
    pub fn new(incoming: Compat01As03<Incoming>, stream_tx: UnboundedSender<TcpStream>) -> Self {
        Inner {
            incoming,
            stream_tx,

            in_addrs: Default::default(),
        }
    }
}

impl Future for Inner {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {
        macro_rules! incoming_ready {
            ($poll:expr) => {
                match $poll {
                    Poll::Pending => break,
                    Poll::Ready(Some(v)) => v,
                    Poll::Ready(None) => {
                        error!("listen closed");
                        return Poll::Ready(());
                    }
                }
            };
        }

        loop {
            let incoming = &mut self.as_mut().incoming;
            pin_mut!(incoming);

            let stream = match incoming_ready!(incoming.poll_next(ctx)) {
                Ok(stream) => stream,
                Err(err) => {
                    error!("incoming err {}", err);
                    return Poll::Ready(());
                }
            };

            if let Ok(sock_addr) = stream.peer_addr() {
                // Check socket address
                if self.in_addrs.contains(&sock_addr.ip()) {
                    info!("ignore duplicate connection {}", sock_addr);
                    continue;
                }

                self.in_addrs.insert(sock_addr.ip());
            }

            // Spawn and read payload
            self.stream_tx.unbounded_send(stream).expect("send stream");
        }

        Poll::Pending
    }
}

pub struct TokioNode {
    gossip:    TokioGossip,
    stream_rx: Option<UnboundedReceiver<TcpStream>>,
}

impl TokioNode {
    pub fn build(config: &Config) -> Self {
        let (stream_tx, stream_rx) = unbounded();
        let incoming = TcpListener::bind(&config.listen)
            .expect("listen")
            .incoming()
            .compat();

        runtime::spawn(Inner::new(incoming, stream_tx));

        let nodes = config.tokio.as_ref().expect("tokio").nodes.clone();
        let gossip = TokioGossip::from(nodes);

        TokioNode {
            gossip,
            stream_rx: Some(stream_rx),
        }
    }

    pub fn handle(&mut self) -> TokioGossip {
        self.gossip.clone()
    }

    pub fn listen(&mut self, _: SocketAddr) {
        // noop, already listen on config.listen
    }

    pub fn register<M>(
        &mut self,
        _: &str,
        handler: impl MessageHandler<Message = M>,
    ) -> ProtocolResult<()>
    where
        M: MessageCodec + Unpin,
    {
        let stream_rx = self.stream_rx.take().expect("no reactor");
        let reactor = Recorder::new(stream_rx, Arc::new(handler));

        runtime::spawn(reactor);
        Ok(())
    }
}

impl Future for TokioNode {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _ctx: &mut Context) -> Poll<Self::Output> {
        Poll::Pending
    }
}
