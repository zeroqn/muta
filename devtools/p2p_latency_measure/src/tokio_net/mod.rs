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
    executor::block_on,
    pin_mut,
};
use log::{error, info};
use protocol::{
    traits::{MessageCodec, MessageHandler},
    ProtocolResult,
};
use tokio::net::{TcpListener, TcpStream};

use crate::config::Config;

const DEFAUTL_LISTEN: &str = "0.0.0.0:9999";

pub struct TokioNode {
    listener: TcpListener,

    gossip: TokioGossip,

    // Save ip addr, make sure no duplicate ip
    in_addrs: HashSet<IpAddr>,

    // Inbound stream deliver, send stream to recorder
    stream_tx: UnboundedSender<TcpStream>,

    stream_rx: Option<UnboundedReceiver<TcpStream>>,
}

impl TokioNode {
    pub fn build(config: &Config) -> Self {
        let nodes = config.tokio.as_ref().expect("tokio").nodes.clone();

        let listener = block_on(TcpListener::bind(DEFAUTL_LISTEN)).expect("listen default");
        let (stream_tx, stream_rx) = unbounded();
        let gossip = TokioGossip::from(nodes);

        TokioNode {
            listener,

            gossip,

            in_addrs: Default::default(),

            stream_tx,
            stream_rx: Some(stream_rx),
        }
    }

    pub fn handle(&mut self) -> TokioGossip {
        self.gossip.clone()
    }

    pub fn listen(&mut self, socket_addr: SocketAddr) {
        self.listener = block_on(TcpListener::bind(&socket_addr)).expect("listen");
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

    fn poll(mut self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {
        macro_rules! incoming_ready {
            ($poll:expr) => {
                match $poll {
                    Poll::Pending => break,
                    Poll::Ready(Ok(v)) => v,
                    Poll::Ready(Err(err)) => {
                        error!("listen incoming err {}", err);
                        return Poll::Ready(());
                    }
                }
            };
        }

        loop {
            let (stream, sock_addr) = {
                let listener = &mut self.as_mut().listener;
                let accept = listener.accept();
                pin_mut!(accept);

                incoming_ready!(accept.poll(ctx))
            };

            // Check socket address
            if self.in_addrs.contains(&sock_addr.ip()) {
                info!("ignore duplicate connection {}", sock_addr);
                continue;
            }

            self.in_addrs.insert(sock_addr.ip());

            // Spawn and read payload
            self.stream_tx.unbounded_send(stream).expect("send stream");
        }

        Poll::Pending
    }
}
