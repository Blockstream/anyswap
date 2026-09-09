#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Error;
use bitcoin::{
    Address,
    hashes::{Hash, sha256},
    secp256k1::{PublicKey, SecretKey},
};

use crate::types::Chain;

/// Returns the current time as seconds since the Unix epoch. On wasm32 the
/// system clock is absent and the browser's is read instead.
#[cfg(not(target_arch = "wasm32"))]
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is broken")
        .as_secs()
}

/// Returns the current time as seconds since the Unix epoch. On wasm32 the
/// system clock is absent and the browser's is read instead.
#[cfg(target_arch = "wasm32")]
pub fn now() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

/// Generates a fresh keypair using the process-wide secp256k1 context.
pub fn generate_keypair() -> (SecretKey, PublicKey) {
    crate::secp().generate_keypair(&mut bitcoin::secp256k1::rand::thread_rng())
}

/// Returns the SHA-256 hash of `preimage` as a 32-byte array.
pub fn sha256_payment_hash(preimage: &[u8]) -> [u8; 32] {
    sha256::Hash::hash(preimage).to_byte_array()
}

pub fn validate_user_address(
    field: &'static str,
    chain: Chain,
    address: &str,
    network: crate::types::SwapNetwork,
) -> Result<(), Error> {
    match chain {
        Chain::Bitcoin => {
            address
                .parse::<Address<bitcoin::address::NetworkUnchecked>>()
                .map_err(|e| anyhow::anyhow!("invalid {field}: {e}"))?
                .require_network(network.bitcoin_network())
                .map_err(|e| anyhow::anyhow!("invalid {field}: {e}"))?;
        }
        Chain::Liquid => {
            let address = address
                .parse::<elements::Address>()
                .map_err(|e| anyhow::anyhow!("invalid {field}: {e}"))?;
            if address.params != network.liquid_address_params() {
                return Err(anyhow::anyhow!("invalid {field}: wrong Liquid network"));
            }
            if !address.is_blinded() {
                return Err(anyhow::anyhow!(
                    "invalid {field}: Liquid address must be confidential"
                ));
            }
        }
    }
    Ok(())
}
