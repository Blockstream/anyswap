use anyswap_sdk::{transaction::liquid::decode_tx_hex, utxo::SwapUtxo as SdkSwapUtxo};
use wasm_bindgen::prelude::*;

use crate::to_js;

/// An output of a swap script, as an indexer reports it. `SwapIn.outputs` and
/// `SwapOut.verifyOpening` return these, and the builders take them back.
#[wasm_bindgen]
#[derive(Clone)]
pub struct SwapUtxo {
    inner: SdkSwapUtxo,
}

#[wasm_bindgen]
impl SwapUtxo {
    /// A Bitcoin output from your own indexer. A value that disagrees with the
    /// funding transaction produces a transaction that will not relay.
    pub fn bitcoin(txid: &str, vout: u32, value: u64) -> SwapUtxo {
        SwapUtxo {
            inner: SdkSwapUtxo::bitcoin(txid, vout, value),
        }
    }

    /// A Liquid output at `vout` of `txHex`, whose value stays blinded inside it.
    /// Wraps the output as-is; the spend builders re-validate script and asset.
    pub fn liquid(tx_hex: &str, vout: u32) -> Result<SwapUtxo, JsError> {
        let tx = decode_tx_hex(tx_hex).map_err(to_js)?;
        let output = tx
            .output
            .get(vout as usize)
            .cloned()
            .ok_or_else(|| JsError::new(&format!("vout {vout} out of range")))?;
        Ok(SwapUtxo {
            inner: SdkSwapUtxo::liquid(tx.txid().to_string(), vout, output),
        })
    }

    /// A copy of this utxo. The builders take ownership of what they are given,
    /// so clone a utxo to use it again after passing it to one.
    #[wasm_bindgen(js_name = clone)]
    pub fn js_clone(&self) -> SwapUtxo {
        self.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn txid(&self) -> String {
        self.inner.txid.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn vout(&self) -> u32 {
        self.inner.vout
    }

    /// `"bitcoin"` or `"liquid"`.
    #[wasm_bindgen(getter)]
    pub fn chain(&self) -> String {
        self.inner.chain().to_string()
    }
}

impl SwapUtxo {
    pub(crate) fn new(inner: SdkSwapUtxo) -> Self {
        Self { inner }
    }

    pub(crate) fn inner(&self) -> &SdkSwapUtxo {
        &self.inner
    }
}

/// The result of `SwapIn.partitionEconomical`.
#[wasm_bindgen]
pub struct EconomicalSplit {
    economical: Vec<SwapUtxo>,
    dust: Vec<SwapUtxo>,
}

#[wasm_bindgen]
impl EconomicalSplit {
    /// UTXOs worth sweeping. Pass these to a refund.
    #[wasm_bindgen(getter)]
    pub fn economical(&self) -> Vec<SwapUtxo> {
        self.economical.clone()
    }

    /// UTXOs worth less than their own fee. Show them rather than sweeping them.
    #[wasm_bindgen(getter)]
    pub fn dust(&self) -> Vec<SwapUtxo> {
        self.dust.clone()
    }
}

impl EconomicalSplit {
    pub(crate) fn new(economical: Vec<SdkSwapUtxo>, dust: Vec<SdkSwapUtxo>) -> Self {
        Self {
            economical: economical.into_iter().map(SwapUtxo::new).collect(),
            dust: dust.into_iter().map(SwapUtxo::new).collect(),
        }
    }
}

/// Unwraps the utxos the builders take.
pub(crate) fn to_utxos(utxos: &[SwapUtxo]) -> Vec<SdkSwapUtxo> {
    utxos.iter().map(|utxo| utxo.inner.clone()).collect()
}
