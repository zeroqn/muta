use std::time::Instant;

use protocol::Bytes;
use tentacle::context::ProtocolContextMutRef;
use tentacle::traits::SessionProtocol;

use crate::compression::Snappy;
use crate::reactor::{MessageRouter, RemotePeer};

use super::message::{ReceivedMessage, SeqChunkMessage};
use super::{DATA_SEQ_TIMEOUT, MAX_CHUNK_SIZE};

pub struct TransmitterProtocol {
    router:             MessageRouter<Snappy>,
    data_buf:           Vec<u8>,
    current_data_seq:   u64,
    first_seq_bytes_at: Instant,
}

impl TransmitterProtocol {
    pub fn new(router: MessageRouter<Snappy>) -> Self {
        TransmitterProtocol {
            router,
            data_buf: Vec::new(),
            current_data_seq: 0,
            first_seq_bytes_at: Instant::now(),
        }
    }
}

impl SessionProtocol for TransmitterProtocol {
    fn received(&mut self, ctx: ProtocolContextMutRef, data: Bytes) {
        let peer_id = match ctx.session.remote_pubkey.as_ref() {
            Some(pk) => pk.peer_id(),
            None => {
                // Dont care result here, connection/keeper will also handle this.
                let _ = ctx.disconnect(ctx.session.id);
                return;
            }
        };
        let session_id = ctx.session.id;

        // Seq u64 takes 8 bytes, and eof bool take 1 byte, so a valid data length
        // must be bigger or equal than 10.
        if data.len() < 10 {
            log::warn!("session {} data size < 10, drop it", session_id);
            return;
        }

        let SeqChunkMessage { seq, eof, data } = SeqChunkMessage::decode(data);
        log::debug!("recived seq {} eof {} data size {}", seq, eof, data.len());

        if data.len() > MAX_CHUNK_SIZE {
            log::warn!(
                "session {} data size > {}, drop it",
                session_id,
                MAX_CHUNK_SIZE
            );

            return;
        }

        if seq == self.current_data_seq {
            if self.first_seq_bytes_at.elapsed() > DATA_SEQ_TIMEOUT {
                log::warn!(
                    "session {} data seq {} timeout, drop it",
                    session_id,
                    self.current_data_seq
                );

                self.data_buf.clear();
                return;
            }

            self.data_buf.extend(data.as_ref());
            log::debug!("data buf size {}", self.data_buf.len());
        } else {
            log::debug!("new data seq {}", seq);

            self.current_data_seq = seq;
            self.data_buf.clear();
            self.data_buf.extend(data.as_ref());
            self.data_buf.shrink_to_fit();
            self.first_seq_bytes_at = Instant::now();
        }

        if !eof {
            return;
        }

        let data = std::mem::replace(&mut self.data_buf, Vec::new());
        log::debug!("final seq {} data size {}", seq, data.len());

        let remote_peer = match RemotePeer::from_proto_context(&ctx) {
            Ok(peer) => peer,
            Err(_err) => {
                log::warn!("received data from unencrypted peer, impossible, drop it");
                return;
            }
        };

        let recv_msg = ReceivedMessage {
            session_id,
            peer_id,
            data: Bytes::from(data),
        };

        tokio::spawn(self.router.route_message(remote_peer, recv_msg));
    }
}
