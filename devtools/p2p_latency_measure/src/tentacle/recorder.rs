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
use tentacle::bytes::Bytes;

pub struct Recorder<M, H> {
    msg_rx:      UnboundedReceiver<Bytes>,
    msg_handler: Arc<H>,

    pin_m: PhantomData<M>,
}

impl<M, H> Recorder<M, H>
where
    H: MessageHandler<Message = M>,
    M: MessageCodec + Unpin,
{
    pub fn new(msg_rx: UnboundedReceiver<Bytes>, msg_handler: Arc<H>) -> Self {
        Recorder {
            msg_rx,
            msg_handler,

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
        macro_rules! break_ready {
            ($poll:expr) => {
                match $poll {
                    Poll::Pending => break,
                    Poll::Ready(Some(v)) => v,
                    Poll::Ready(None) => return Poll::Ready(()),
                }
            };
        }

        loop {
            let msg_rx = &mut self.as_mut().msg_rx;
            pin_mut!(msg_rx);

            let msg_bytes = break_ready!(msg_rx.poll_next(ctx));
            let handler = Arc::clone(&self.msg_handler);

            runtime::spawn(async move {
                let ctx = Context::new();

                if let Ok(msg) = M::decode(msg_bytes).await {
                    if let Err(err) = handler.process(ctx, msg).await {
                        error!("msg handler: {}", err);
                    }
                }
            });
        }

        Poll::Pending
    }
}
