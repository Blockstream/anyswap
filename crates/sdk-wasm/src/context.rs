use anyswap_sdk::{context::SwapContext as SdkSwapContext, wallet::Wallet as SdkWallet};
use wasm_bindgen::prelude::*;

use crate::{to_js, wallet::Wallet};

/// The wallet and chain parameters shared by every swap of one client. Build one
/// at startup and pass it to the swap methods. Liquid swaps need
/// `liquidGenesisHash` and `liquidPolicyAsset`, read from your own chain source.
#[wasm_bindgen]
pub struct SwapContext {
    inner: SdkSwapContext<SdkWallet>,
}

#[wasm_bindgen]
impl SwapContext {
    #[wasm_bindgen(constructor)]
    pub fn new(
        wallet: &Wallet,
        liquid_genesis_hash: Option<String>,
        liquid_policy_asset: Option<String>,
    ) -> Result<SwapContext, JsError> {
        let inner = SdkSwapContext::new(wallet.inner().clone());
        let inner = match (liquid_genesis_hash, liquid_policy_asset) {
            (Some(genesis_hash), Some(policy_asset)) => inner
                .with_liquid(&genesis_hash, &policy_asset)
                .map_err(to_js)?,
            (None, None) => inner,
            _ => {
                return Err(JsError::new(
                    "Liquid needs both a genesis hash and a policy asset",
                ));
            }
        };
        Ok(SwapContext { inner })
    }

    /// Network this client operates on.
    #[wasm_bindgen(getter)]
    pub fn network(&self) -> String {
        self.inner.network().to_string()
    }
}

impl SwapContext {
    pub(crate) fn inner(&self) -> &SdkSwapContext<SdkWallet> {
        &self.inner
    }
}
