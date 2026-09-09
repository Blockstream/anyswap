//! AnySwap client SDK.
//!
//! Client-side layer over [`anyswap_core`]: typed swap views
//! ([`swap_in::SwapIn`], [`swap_out::SwapOut`]), string codecs shared with the
//! Python/WASM bindings ([`codec`]), per-swap key derivation ([`wallet::Wallet`]),
//! and an HTTP client for the server API ([`client::HttpClient`], behind the
//! `client` feature; its WebSocket subscription, [`ws`], behind the native-only
//! `ws` feature).
//!
//! On-chain primitives (swap script, transaction builders, swap_fee and invoice
//! parsing) are re-exported from [`anyswap_core`] so consumers depend on this
//! crate alone.

#[cfg(feature = "esplora")]
pub use anyswap_core::esplora;
pub use anyswap_core::{api, auth, fee, invoice, script, transaction, types};
pub use bitcoin;
pub use elements;

#[cfg(feature = "client")]
pub mod client;
pub use anyswap_core::codec;
pub mod context;
#[cfg(feature = "driver")]
pub mod driver;
pub mod swap_in;
pub mod swap_out;
pub mod swap_script;
pub mod utxo;
pub mod wallet;
#[cfg(feature = "ws")]
pub mod ws;
