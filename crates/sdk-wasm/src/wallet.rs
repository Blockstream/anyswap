use anyswap_sdk::{
    codec,
    wallet::{SwapWallet, SwapWalletExt, Wallet as SdkWallet},
};
use bip39::Mnemonic;
use wasm_bindgen::prelude::*;

use crate::{parse_network, to_js};

/// BIP39/BIP32 HD wallet deriving per-swap keys from a `swap_id`. Holds no
/// balances - deposits go directly to the swap script address.
#[wasm_bindgen]
pub struct Wallet {
    inner: SdkWallet,
}

#[wasm_bindgen]
impl Wallet {
    /// Restores a wallet from an existing mnemonic for the given network.
    #[wasm_bindgen(js_name = fromMnemonic)]
    pub fn from_mnemonic(mnemonic: &str, network: &str) -> Result<Wallet, JsError> {
        let mnemonic = Mnemonic::parse(mnemonic).map_err(to_js)?;
        let network = parse_network(network)?;
        Ok(Wallet {
            inner: SdkWallet::from_mnemonic(&mnemonic, network).map_err(to_js)?,
        })
    }

    /// Stable pseudonymous identifier sent to the server (hex SHA-256 of the
    /// identity pubkey).
    #[wasm_bindgen(getter, js_name = userId)]
    pub fn user_id(&self) -> Result<String, JsError> {
        self.inner.user_id().map_err(to_js)
    }

    /// Network this wallet was created for.
    #[wasm_bindgen(getter)]
    pub fn network(&self) -> String {
        self.inner.network().to_string()
    }

    /// Stable identity pubkey, hex-encoded.
    #[wasm_bindgen(js_name = identityPubkey)]
    pub fn identity_pubkey(&self) -> Result<String, JsError> {
        Ok(hex::encode(
            self.inner.identity_pubkey().map_err(to_js)?.serialize(),
        ))
    }

    /// Stable identity private key used to authenticate API requests, hex-encoded.
    #[wasm_bindgen(js_name = identityPrivkey)]
    pub fn identity_privkey(&self) -> Result<String, JsError> {
        Ok(hex::encode(
            self.inner.identity_privkey().map_err(to_js)?.secret_bytes(),
        ))
    }

    /// Source refund public key (swap-in), hex-encoded.
    #[wasm_bindgen(js_name = sourceRefundPubkey)]
    pub fn source_refund_pubkey(&self, swap_id: &str) -> Result<String, JsError> {
        let id = codec::parse_uuid(swap_id).map_err(to_js)?;
        Ok(hex::encode(
            self.inner
                .source_refund_pubkey(&id)
                .map_err(to_js)?
                .serialize(),
        ))
    }

    /// Source refund private key (swap-in), hex-encoded.
    #[wasm_bindgen(js_name = sourceRefundPrivkey)]
    pub fn source_refund_privkey(&self, swap_id: &str) -> Result<String, JsError> {
        let id = codec::parse_uuid(swap_id).map_err(to_js)?;
        Ok(hex::encode(
            self.inner
                .source_refund_privkey(&id)
                .map_err(to_js)?
                .secret_bytes(),
        ))
    }

    /// Destination claim public key (swap-out), hex-encoded.
    #[wasm_bindgen(js_name = destClaimPubkey)]
    pub fn dest_claim_pubkey(&self, swap_id: &str) -> Result<String, JsError> {
        let id = codec::parse_uuid(swap_id).map_err(to_js)?;
        Ok(hex::encode(
            self.inner
                .dest_claim_pubkey(&id)
                .map_err(to_js)?
                .serialize(),
        ))
    }

    /// Destination claim private key (swap-out), hex-encoded.
    #[wasm_bindgen(js_name = destClaimPrivkey)]
    pub fn dest_claim_privkey(&self, swap_id: &str) -> Result<String, JsError> {
        let id = codec::parse_uuid(swap_id).map_err(to_js)?;
        Ok(hex::encode(
            self.inner
                .dest_claim_privkey(&id)
                .map_err(to_js)?
                .secret_bytes(),
        ))
    }

    /// Source-side blinding public key (Liquid swap-in), hex-encoded.
    #[wasm_bindgen(js_name = sourceBlindingPubkey)]
    pub fn source_blinding_pubkey(&self, swap_id: &str) -> Result<String, JsError> {
        let id = codec::parse_uuid(swap_id).map_err(to_js)?;
        Ok(hex::encode(
            self.inner
                .source_blinding_pubkey(&id)
                .map_err(to_js)?
                .serialize(),
        ))
    }

    /// Source-side blinding private key (Liquid swap-in), hex-encoded.
    #[wasm_bindgen(js_name = sourceBlindingPrivkey)]
    pub fn source_blinding_privkey(&self, swap_id: &str) -> Result<String, JsError> {
        let id = codec::parse_uuid(swap_id).map_err(to_js)?;
        Ok(hex::encode(
            self.inner
                .source_blinding_privkey(&id)
                .map_err(to_js)?
                .secret_bytes(),
        ))
    }

    /// Destination-side blinding public key (Liquid swap-out), hex-encoded.
    #[wasm_bindgen(js_name = destBlindingPubkey)]
    pub fn dest_blinding_pubkey(&self, swap_id: &str) -> Result<String, JsError> {
        let id = codec::parse_uuid(swap_id).map_err(to_js)?;
        Ok(hex::encode(
            self.inner
                .dest_blinding_pubkey(&id)
                .map_err(to_js)?
                .serialize(),
        ))
    }

    /// Destination-side blinding private key (Liquid swap-out), hex-encoded.
    #[wasm_bindgen(js_name = destBlindingPrivkey)]
    pub fn dest_blinding_privkey(&self, swap_id: &str) -> Result<String, JsError> {
        let id = codec::parse_uuid(swap_id).map_err(to_js)?;
        Ok(hex::encode(
            self.inner
                .dest_blinding_privkey(&id)
                .map_err(to_js)?
                .secret_bytes(),
        ))
    }

    pub fn preimage(&self, swap_id: &str) -> Result<String, JsError> {
        let id = codec::parse_uuid(swap_id).map_err(to_js)?;
        Ok(hex::encode(self.inner.preimage(&id).map_err(to_js)?))
    }

    #[wasm_bindgen(js_name = paymentHash)]
    pub fn payment_hash(&self, swap_id: &str) -> Result<String, JsError> {
        let id = codec::parse_uuid(swap_id).map_err(to_js)?;
        Ok(hex::encode(self.inner.payment_hash(&id).map_err(to_js)?))
    }
}

impl Wallet {
    pub(crate) fn inner(&self) -> &SdkWallet {
        &self.inner
    }
}
