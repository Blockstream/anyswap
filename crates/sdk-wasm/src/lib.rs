//! WASM bindings for the AnySwap client SDK.
//!
//! Thin `wasm-bindgen` wrapper over [`anyswap_sdk`] for browser/JS callers. The
//! [`driver`] runs whole swaps: the page hands it a server client, a chain
//! client and a store written in JS, and the loop runs in Rust over them.
//! Beside it are the key derivation it runs on ([`wallet::Wallet`]), the
//! client-wide [`context::SwapContext`], request signing ([`auth`]), and free
//! functions for BOLT11 invoice parsing, the swap fee, mnemonic handling, swap
//! id minting and the protocol version.
//!
//! HTTP is out of scope: the browser does its own `fetch`.

use std::str::FromStr;

use anyswap_sdk::types::SwapNetwork;
use wasm_bindgen::prelude::*;

pub mod auth;
pub mod config;
pub mod context;
#[cfg(target_arch = "wasm32")]
pub mod driver;
pub mod wallet;

#[wasm_bindgen(start)]
fn start() {
    console_error_panic_hook::set_once();
}

/// Maps any error into a JS exception carrying its `Display` message.
pub(crate) fn to_js(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

/// Parses a network name (`Mainnet`, `Testnet`, `Signet`, `Regtest`).
pub(crate) fn parse_network(s: &str) -> Result<SwapNetwork, JsError> {
    SwapNetwork::from_str(s).map_err(|_| {
        JsError::new(&format!(
            "unknown network {s:?}; expected Mainnet, Testnet, Signet, or Regtest"
        ))
    })
}

#[wasm_bindgen(js_name = paymentHashFromInvoice)]
pub fn payment_hash_from_invoice(bolt11: &str) -> Result<String, JsError> {
    let hash = anyswap_sdk::invoice::payment_hash(bolt11).map_err(to_js)?;
    Ok(hex::encode(hash))
}

#[wasm_bindgen(js_name = amountMsatFromInvoice)]
pub fn amount_msat_from_invoice(bolt11: &str) -> Result<u64, JsError> {
    anyswap_sdk::invoice::amount_msat(bolt11).map_err(to_js)
}

#[wasm_bindgen(js_name = expiryAtFromInvoice)]
pub fn expiry_at_from_invoice(bolt11: &str) -> Result<f64, JsError> {
    Ok(anyswap_sdk::invoice::expiry_at(bolt11).map_err(to_js)? as f64)
}

#[wasm_bindgen(js_name = networkFromInvoice)]
pub fn network_from_invoice(bolt11: &str) -> Result<String, JsError> {
    Ok(anyswap_sdk::invoice::network(bolt11)
        .map_err(to_js)?
        .to_string())
}

/// The total swap fee: `base_fee + receive_amount * service_ppm / 1_000_000`, saturating at
/// `u64::MAX`.
#[wasm_bindgen(js_name = swapFee)]
pub fn swap_fee(receive_amount: u64, service_ppm: u32, base_fee: u64) -> u64 {
    anyswap_sdk::fee::swap_fee(receive_amount, service_ppm.into(), base_fee)
}

/// Generates a random mnemonic; `word_count` must be 12, 15, 18, 21, or 24.
#[wasm_bindgen(js_name = generateMnemonic)]
pub fn generate_mnemonic(word_count: u8) -> Result<String, JsError> {
    Ok(anyswap_sdk::wallet::Wallet::generate_mnemonic(word_count)
        .map_err(to_js)?
        .to_string())
}

#[wasm_bindgen(js_name = validateMnemonic)]
pub fn validate_mnemonic(mnemonic: &str) -> bool {
    bip39::Mnemonic::parse(mnemonic).is_ok()
}

/// Mints a fresh random swap id (UUID v4) for a new swap.
#[wasm_bindgen(js_name = newSwapId)]
pub fn new_swap_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The protocol version this SDK speaks, for a create request's
/// `protocol_version`.
#[wasm_bindgen(js_name = protocolVersion)]
pub fn protocol_version() -> String {
    anyswap_sdk::PROTOCOL_VERSION.into()
}
