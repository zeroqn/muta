use crate::tokio_net::{Kind, TokioError};
use tentacle::bytes::Bytes;

pub struct Packet {
    pub length: u32,
    pub crc:    u32,
    pub bytes:  Bytes,
}

impl Packet {
    pub fn length(bytes: &[u8]) -> Result<u32, TokioError> {
        if bytes.len() < 4 {
            return Err(TokioError::CorruptedHeader(Kind::HeaderNoLength));
        }

        let mut buf = [0u8; 4];
        buf.copy_from_slice(&bytes[..4]);

        Ok(u32::from_be_bytes(buf))
    }

    pub fn crc32(bytes: &[u8]) -> Result<u32, TokioError> {
        if bytes.len() < 8 {
            return Err(TokioError::CorruptedHeader(Kind::HeaderNoCrc));
        }

        let mut buf = [0u8; 4];
        buf.copy_from_slice(&bytes[4..8]);

        Ok(u32::from_be_bytes(buf))
    }

    pub fn verify(&self) -> Result<(), TokioError> {
        let length = Packet::length(&self.bytes);
        let crc32 = Packet::crc32(&self.bytes);
    }
}
