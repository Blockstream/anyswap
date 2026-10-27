use anyswap_sdk::{
    auth::{AuthCore, AuthRequest},
    bitcoin::secp256k1::{Keypair, Secp256k1},
    codec,
    wallet::{SwapWallet, SwapWalletExt, Wallet as SdkWallet},
};
use bip39::Mnemonic;
use wasm_bindgen::prelude::*;
use zeroize::Zeroizing;

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
    pub fn from_mnemonic(mnemonic: String, network: &str) -> Result<Wallet, JsError> {
        // Owned, so the copy of the words passed in from JS is wiped on return.
        let words = Zeroizing::new(mnemonic);
        let mnemonic = Mnemonic::parse(words.as_str()).map_err(to_js)?;
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

    /// Schnorr-signs the AnySwap auth digest of a request with the identity
    /// key, which never leaves the wallet. Returns the 64-byte signature
    /// hex-encoded. `target` is the request path with its query string, minus
    /// the query when it carries the auth fields.
    #[wasm_bindgen(js_name = signRequest)]
    pub fn sign_request(
        &self,
        method: &str,
        target: &str,
        raw_body: &[u8],
        swap_id: &str,
        timestamp: i64,
    ) -> Result<String, JsError> {
        let secret = self.inner.identity_privkey().map_err(to_js)?;
        let keypair = Keypair::from_secret_key(&Secp256k1::new(), &secret);
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
}

impl Wallet {
    pub(crate) fn inner(&self) -> &SdkWallet {
        &self.inner
    }
}
