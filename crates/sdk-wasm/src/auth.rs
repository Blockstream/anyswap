use anyswap_sdk::{
    auth::{AuthCore, AuthRequest},
    bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey},
};
use wasm_bindgen::prelude::*;

/// Hashes the body, builds the AnySwap auth digest, and Schnorr-signs it.
/// Returns the 64-byte signature hex-encoded. `target` is the request path with
/// its query string, minus the query when it carries the auth fields.
#[wasm_bindgen(js_name = signRequest)]
pub fn sign_request(
    identity_privkey: &str,
    method: &str,
    target: &str,
    raw_body: &[u8],
    swap_id: &str,
    timestamp: i64,
) -> Result<String, JsError> {
    let keypair = keypair_from_hex(identity_privkey)?;

    let signature = AuthCore::sign_request(
        &AuthRequest {
            method: &method.to_ascii_uppercase(),
            target,
            body: raw_body,
            swap_id,
            timestamp,
        },
        &keypair,
    );
    Ok(hex::encode(signature.as_ref()))
}

#[wasm_bindgen(js_name = getPubkey)]
pub fn get_pubkey(identity_privkey: &str) -> Result<String, JsError> {
    let keypair = keypair_from_hex(identity_privkey)?;

    Ok(hex::encode(keypair.public_key().serialize()))
}

fn keypair_from_hex(identity_privkey: &str) -> Result<Keypair, JsError> {
    let secret_bytes = hex::decode(identity_privkey.trim_start_matches("0x"))
        .map_err(|e| JsError::new(&format!("invalid identity private key hex: {e}")))?;
    let secret_key = SecretKey::from_slice(&secret_bytes)
        .map_err(|e| JsError::new(&format!("invalid identity private key: {e}")))?;
    Ok(Keypair::from_secret_key(&Secp256k1::new(), &secret_key))
}
