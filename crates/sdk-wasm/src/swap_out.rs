use anyswap_sdk::{
    api::SwapState,
    swap_out::{SwapOut as SdkSwapOut, SwapOutStatus},
};
use wasm_bindgen::prelude::*;

use crate::{
    context::SwapContext,
    to_js,
    utxo::{self, SwapUtxo},
};

/// Typed client view of a swap-out (Lightning to on-chain). The user pays a held
/// claim invoice and holds the claim key/preimage; the server locks the proceeds
/// into the swap script and holds the refund key.
#[wasm_bindgen]
pub struct SwapOut {
    inner: SdkSwapOut,
}

#[wasm_bindgen]
impl SwapOut {
    /// Parses a server swap-state object (e.g. `JSON.parse` of the `/v1/swap/{id}`
    /// response) into a typed swap-out.
    #[wasm_bindgen(js_name = fromResponse)]
    pub fn from_response(val: JsValue) -> Result<SwapOut, JsError> {
        let state: SwapState = serde_wasm_bindgen::from_value(val)
            .map_err(|e| JsError::new(&format!("invalid swap-out response: {e}")))?;
        Ok(SwapOut {
            inner: SdkSwapOut::try_from(state).map_err(to_js)?,
        })
    }

    #[wasm_bindgen(getter, js_name = swapId)]
    pub fn swap_id(&self) -> String {
        self.inner.swap_id.to_string()
    }

    #[wasm_bindgen(getter, js_name = protocolVersion)]
    pub fn protocol_version(&self) -> String {
        self.inner.protocol_version.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn chain(&self) -> String {
        self.inner.chain.to_string()
    }

    #[wasm_bindgen(getter)]
    pub fn asset(&self) -> String {
        self.inner.asset.clone()
    }

    #[wasm_bindgen(getter, js_name = receiveAmount)]
    pub fn receive_amount(&self) -> u64 {
        self.inner.receive_amount
    }

    #[wasm_bindgen(getter, js_name = sendAmount)]
    pub fn send_amount(&self) -> u64 {
        self.inner.send_amount
    }

    #[wasm_bindgen(getter, js_name = baseFee)]
    pub fn base_fee(&self) -> u64 {
        self.inner.base_fee
    }

    #[wasm_bindgen(getter, js_name = serviceFee)]
    pub fn service_fee(&self) -> u64 {
        self.inner.service_fee
    }

    #[wasm_bindgen(getter, js_name = paymentHash)]
    pub fn payment_hash(&self) -> String {
        hex::encode(self.inner.payment_hash)
    }

    #[wasm_bindgen(getter)]
    pub fn csv(&self) -> u32 {
        self.inner.csv
    }

    /// User's pubkey in the swap script (claim side), hex-encoded.
    #[wasm_bindgen(getter, js_name = claimPubkey)]
    pub fn claim_pubkey(&self) -> String {
        hex::encode(self.inner.claim_pubkey.serialize())
    }

    /// Server's pubkey in the swap script (refund side), hex-encoded.
    #[wasm_bindgen(getter, js_name = refundPubkey)]
    pub fn refund_pubkey(&self) -> String {
        hex::encode(self.inner.refund_pubkey.serialize())
    }

    /// User's on-chain destination for a successful claim.
    #[wasm_bindgen(getter, js_name = claimAddress)]
    pub fn claim_address(&self) -> String {
        self.inner.claim_address.clone()
    }

    #[wasm_bindgen(getter, js_name = claimInvoice)]
    pub fn claim_invoice(&self) -> String {
        self.inner.claim_invoice.clone()
    }

    #[wasm_bindgen(getter, js_name = createdAt)]
    pub fn created_at(&self) -> f64 {
        self.inner.created_at as f64
    }

    #[wasm_bindgen(getter, js_name = transitionedAt)]
    pub fn transitioned_at(&self) -> f64 {
        self.inner.transitioned_at as f64
    }

    #[wasm_bindgen(getter)]
    pub fn status(&self) -> String {
        match self.inner.status {
            SwapOutStatus::AwaitClaimInvoicePay => "AwaitClaimInvoicePay",
            SwapOutStatus::OpeningTxBroadcast => "OpeningTxBroadcast",
            SwapOutStatus::AwaitPreimageRevealed { .. } => "AwaitPreimageRevealed",
            SwapOutStatus::AwaitRefundTxConfirmation { .. } => "AwaitRefundTxConfirmation",
            SwapOutStatus::Claimed { .. } => "Claimed",
            SwapOutStatus::Canceled { .. } => "Canceled",
            SwapOutStatus::Refunded { .. } => "Refunded",
        }
        .to_owned()
    }

    /// Opening transaction id, once the server has broadcast the deposit.
    #[wasm_bindgen(getter, js_name = openingTxId)]
    pub fn opening_tx_id(&self) -> Option<String> {
        self.inner.status.outpoint().map(|o| o.txid.to_string())
    }

    /// Opening transaction output index, once the server has broadcast the deposit.
    #[wasm_bindgen(getter, js_name = openingTxOut)]
    pub fn opening_tx_out(&self) -> Option<u32> {
        self.inner.status.outpoint().map(|o| o.vout)
    }

    /// Server's revealed refund privkey (hex), available only after `Claimed`,
    /// enabling a cheap cooperative keypath claim.
    #[wasm_bindgen(getter, js_name = refundPrivkey)]
    pub fn refund_privkey(&self) -> Option<String> {
        match &self.inner.status {
            SwapOutStatus::Claimed { refund_privkey, .. } => {
                Some(hex::encode(refund_privkey.secret_bytes()))
            }
            _ => None,
        }
    }

    #[wasm_bindgen(getter, js_name = cancelMessage)]
    pub fn cancel_message(&self) -> Option<String> {
        match &self.inner.status {
            SwapOutStatus::Canceled { cancel_message, .. } => Some(cancel_message.clone()),
            _ => None,
        }
    }

    /// Machine-readable cancellation reason (e.g. `TxBroadcastFailed`), available
    /// only after `Canceled`.
    #[wasm_bindgen(getter, js_name = errorCode)]
    pub fn error_code(&self) -> Option<String> {
        match &self.inner.status {
            SwapOutStatus::Canceled { error_code, .. } => Some(error_code.to_string()),
            _ => None,
        }
    }

    /// Address the server's opening transaction pays.
    pub fn address(&self, ctx: &SwapContext) -> Result<String, JsError> {
        self.inner.address(ctx.inner()).map_err(to_js)
    }

    /// Checks the server's opening transaction and returns the output to claim.
    /// Call this before releasing the preimage.
    #[wasm_bindgen(js_name = verifyOpening)]
    pub fn verify_opening(&self, ctx: &SwapContext, tx_hex: &str) -> Result<SwapUtxo, JsError> {
        self.inner
            .verify_opening(ctx.inner(), tx_hex)
            .map(SwapUtxo::new)
            .map_err(to_js)
    }

    /// Value of `utxo` in sats, unblinded on Liquid.
    pub fn value(&self, ctx: &SwapContext, utxo: &SwapUtxo) -> Result<u64, JsError> {
        self.inner.value(ctx.inner(), utxo.inner()).map_err(to_js)
    }

    /// Claim of `utxos` revealing the preimage on chain, as signed transaction
    /// hex. `destination` defaults to the swap's claim address.
    #[wasm_bindgen(js_name = claimByPreimage)]
    pub fn claim_by_preimage(
        &self,
        ctx: &SwapContext,
        utxos: Vec<SwapUtxo>,
        destination: Option<String>,
        fee_rate_sat_vb: f64,
    ) -> Result<String, JsError> {
        self.inner
            .claim_by_preimage(
                ctx.inner(),
                &utxo::to_utxos(&utxos),
                destination.as_deref(),
                fee_rate_sat_vb,
            )
            .map_err(to_js)
    }

    /// Cooperative claim of `utxos`, as signed transaction hex. `destination`
    /// defaults to the swap's claim address.
    #[wasm_bindgen(js_name = claimByKeypath)]
    pub fn claim_by_keypath(
        &self,
        ctx: &SwapContext,
        utxos: Vec<SwapUtxo>,
        destination: Option<String>,
        fee_rate_sat_vb: f64,
    ) -> Result<String, JsError> {
        self.inner
            .claim_by_keypath(
                ctx.inner(),
                &utxo::to_utxos(&utxos),
                destination.as_deref(),
                fee_rate_sat_vb,
            )
            .map_err(to_js)
    }
}
