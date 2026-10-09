//! WASM bindings for the AnySwap client SDK.
//!
//! Thin `wasm-bindgen` wrapper over [`anyswap_sdk`] for browser/JS callers:
//! BIP39/BIP32 key derivation ([`wallet::Wallet`]), the client-wide
//! [`context::SwapContext`], and typed swap views with transaction builders
//! ([`swap_in::SwapIn`], [`swap_out::SwapOut`]). Plus free functions for BOLT11
//! invoice parsing, swap_fee calculation, mnemonic handling, swap id minting,
//! and the protocol version.
//!
//! HTTP is intentionally out of scope - the browser does its own `fetch`. Parse
//! the server's swap-state JSON into a JS object and hand it to
//! `SwapIn.fromResponse` / `SwapOut.fromResponse`. The [`driver`] runs whole
//! swaps on the same terms: the page hands it a server client, a chain client
//! and a store written in JS, and the loop runs in Rust over them.

use std::str::FromStr;

use anyswap_sdk::types::SwapNetwork;
use wasm_bindgen::prelude::*;

pub mod auth;
pub mod config;
pub mod context;
#[cfg(target_arch = "wasm32")]
pub mod driver;
pub mod swap_in;
pub mod swap_out;
pub mod utxo;
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

/// The proportional service fee: `receive_amount * service_ppm / 1_000_000`, saturating at
/// `u64::MAX`.
#[wasm_bindgen(js_name = serviceFee)]
pub fn service_fee(receive_amount: u64, service_ppm: u32) -> u64 {
    anyswap_sdk::fee::service_fee(receive_amount, service_ppm.into())
}

/// The least the user must send to receive `receiveAmount`; `undefined` on
/// overflow.
#[wasm_bindgen(js_name = minSendForReceive)]
pub fn min_send_for_receive(receive_amount: u64, service_ppm: u32, base_fee: u64) -> Option<u64> {
    anyswap_sdk::fee::min_send_for_receive(receive_amount, service_ppm.into(), base_fee)
}

/// The most the user can receive for `sendAmount`; `undefined` when it does
/// not cover `baseFee`.
#[wasm_bindgen(js_name = maxReceiveForSend)]
pub fn max_receive_for_send(send_amount: u64, service_ppm: u32, base_fee: u64) -> Option<u64> {
    anyswap_sdk::fee::max_receive_for_send(send_amount, service_ppm.into(), base_fee)
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
