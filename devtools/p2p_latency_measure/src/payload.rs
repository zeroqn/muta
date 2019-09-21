use derive_more::Display;
use lazy_static::lazy_static;
use rand::random;

pub const SIZE_512B: u64 = 512;
pub const SIZE_5K: u64 = 1024 * 5;
pub const SIZE_50K: u64 = 1024 * 50;
pub const SIZE_500K: u64 = 1024 * 500;

// Generate random bytes, avoid compression optimization
lazy_static! {
    // 512B
    pub static ref BYTES_512: Vec<u8> = (0..512).map(|_| random::<u8>()).collect();

    // 5K
    pub static ref BYTES_5K: Vec<u8> = (0..10)
        .map(|_| BYTES_512.as_slice())
        .flatten()
        .cloned()
        .collect();

    // 50K
    pub static ref BYTES_50K: Vec<u8> = (0..10)
        .map(|_| BYTES_5K.as_slice())
        .flatten()
        .cloned()
        .collect();

    // 500K
    pub static ref BYTES_500K: Vec<u8> = (0..10)
        .map(|_| BYTES_50K.as_slice())
        .flatten()
        .cloned()
        .collect();
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Copy, Display)]
pub enum Payload {
    #[display(fmt = "payload: 512B")]
    B512,

    #[display(fmt = "payload: 5K")]
    K5,

    #[display(fmt = "payload: 50K")]
    K50,

    #[display(fmt = "payload: 500K")]
    K500,
}

impl Payload {
    pub fn size(self) -> u64 {
        match self {
            Self::B512 => SIZE_512B,
            Self::K5 => SIZE_5K,
            Self::K50 => SIZE_50K,
            Self::K500 => SIZE_500K,
        }
    }

    pub fn gen(self) -> Vec<u8> {
        match self {
            Self::B512 => BYTES_512.clone(),
            Self::K5 => BYTES_5K.clone(),
            Self::K50 => BYTES_50K.clone(),
            Self::K500 => BYTES_500K.clone(),
        }
    }

    pub fn iter() -> impl Iterator<Item = &'static Payload> {
        [Payload::B512, Payload::K5, Payload::K50, Payload::K500].iter()
    }
}

/// # Panic
impl From<u64> for Payload {
    fn from(size: u64) -> Payload {
        match size {
            SIZE_512B => Payload::B512,
            SIZE_5K => Payload::K5,
            SIZE_50K => Payload::K50,
            SIZE_500K => Payload::K500,
            _ => panic!("unrecognized payload size"),
        }
    }
}
