//! Shared by the CLN plugin (server) and the SDK (client) on-chain primitives for AnySwap.

pub mod api;
pub mod codec;
#[cfg(feature = "esplora")]
pub mod esplora;
pub mod fee;
pub mod invoice;
pub mod script;
pub mod transaction;
pub mod types;
pub mod utils;
pub mod wallet;

pub mod musig;

pub mod auth;

use std::sync::OnceLock;

use bitcoin::secp256k1::{All, Secp256k1};

/// The wire protocol this build speaks: the crate's major and minor version,
/// since a patch release never changes the wire.
pub const PROTOCOL_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION_MAJOR"),
    ".",
    env!("CARGO_PKG_VERSION_MINOR"),
    ".0"
);

/// Shared secp256k1 context, lazily initialized once per process.
pub fn secp() -> &'static Secp256k1<All> {
    static SECP: OnceLock<Secp256k1<All>> = OnceLock::new();
    SECP.get_or_init(Secp256k1::new)
}
