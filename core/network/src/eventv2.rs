use derive_more::Display;

use crate::p2p;

#[derive(Debug, Display)]
pub enum Event {
    #[display(fmt = "{}", _0)]
    P2p(p2p::Event),
}
