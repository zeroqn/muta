use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::Arc,
    task::{Context as TaskContext, Poll},
};

use futures::{channel::mpsc::UnboundedReceiver, pin_mut, stream::Stream};
use log::error;
use protocol::traits::{Context, MessageCodec, MessageHandler};
use tokio::{
    codec::{Framed, LengthDelimitedCodec},
    net::TcpStream,
};

struct Reactor<M, H> {
    framed:  Framed<TcpStream, LengthDelimitedCodec>,
    handler: Arc<H>,

    pin_m: PhantomData<M>,
}

impl<M, H> Reactor<M, H>
where
    H: MessageHandler<Message = M>,
    M: MessageCodec + Unpin,
{
    pub fn new(stream: TcpStream, handler: Arc<H>) -> Self {
        let framed = Framed::new(stream, LengthDelimitedCodec::new());

        Reactor {
            framed,
            handler,

            pin_m: PhantomData,
        }
    }
}

impl<M, H> Future for Reactor<M, H>
where
    H: MessageHandler<Message = M>,
    M: MessageCodec + Unpin,
{
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, ctx: &mut TaskContext) -> Poll<Self::Output> {
        loop {
            let framed = &mut self.as_mut().framed;
            pin_mut!(framed);

            let msg_bytes = match crate::break_ready!(framed.poll_next(ctx)) {
                Ok(bytes) => bytes,
                Err(err) => {
                    error!("framed decode err {}", err);
                    return Poll::Ready(());
                }
            };

            let handler = Arc::clone(&self.handler);

            runtime::spawn(async move {
                let msg = match M::decode(msg_bytes.freeze()).await {
                    Ok(msg) => msg,
                    Err(err) => {
                        error!("msg decode {}", err);
                        return;
                    }
                };

                if let Err(err) = handler.process(Context::new(), msg).await {
                    error!("msg handler: {}", err);
                };
            });
        }

        Poll::Pending
    }
}

pub struct Recorder<M, H> {
    stream_rx: UnboundedReceiver<TcpStream>,
    handler:   Arc<H>,

    pin_m: PhantomData<M>,
}

impl<M, H> Recorder<M, H>
where
    H: MessageHandler<Message = M>,
    M: MessageCodec + Unpin,
{
    pub fn new(stream_rx: UnboundedReceiver<TcpStream>, handler: Arc<H>) -> Self {
        Recorder {
            stream_rx,
            handler,

            pin_m: PhantomData,
        }
    }
}

impl<M, H> Future for Recorder<M, H>
where
    H: MessageHandler<Message = M>,
    M: MessageCodec + Unpin,
{
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, ctx: &mut TaskContext) -> Poll<Self::Output> {
        loop {
            let stream_rx = &mut self.as_mut().stream_rx;
            pin_mut!(stream_rx);

            let stream = crate::break_ready!(stream_rx.poll_next(ctx));

            runtime::spawn(Reactor::new(stream, Arc::clone(&self.handler)));
        }

        Poll::Pending
    }
}
