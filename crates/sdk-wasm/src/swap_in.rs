use anyswap_sdk::{
    api::SwapState,
    swap_in::{SwapIn as SdkSwapIn, SwapInStatus},
};
use wasm_bindgen::prelude::*;

use crate::{
    context::SwapContext,
    to_js,
    utxo::{self, EconomicalSplit, SwapUtxo},
};

/// Typed client view of a swap-in (on-chain to Lightning). The user locks funds
/// into the swap script and holds the refund key; the server pays the claim
/// invoice and holds the claim key.
#[wasm_bindgen]
pub struct SwapIn {
    inner: SdkSwapIn,
}

#[wasm_bindgen]
impl SwapIn {
    /// Parses a server swap-state object (e.g. `JSON.parse` of the `/v1/swap/{id}`
    /// response) into a typed swap-in.
    #[wasm_bindgen(js_name = fromResponse)]
    pub fn from_response(val: JsValue) -> Result<SwapIn, JsError> {
        let state: SwapState = serde_wasm_bindgen::from_value(val)
            .map_err(|e| JsError::new(&format!("invalid swap-in response: {e}")))?;
        Ok(SwapIn {
            inner: SdkSwapIn::try_from(state).map_err(to_js)?,
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

    /// User's pubkey in the swap script (refund side), hex-encoded.
    #[wasm_bindgen(getter, js_name = refundPubkey)]
    pub fn refund_pubkey(&self) -> String {
        hex::encode(self.inner.refund_pubkey.serialize())
    }

    /// Server's pubkey in the swap script (claim side), hex-encoded.
    #[wasm_bindgen(getter, js_name = claimPubkey)]
    pub fn claim_pubkey(&self) -> String {
        hex::encode(self.inner.claim_pubkey.serialize())
    }

    #[wasm_bindgen(getter, js_name = claimInvoice)]
    pub fn claim_invoice(&self) -> String {
        self.inner.claim_invoice.clone()
    }

    /// User's refund destination, when one was given at creation.
    #[wasm_bindgen(getter, js_name = refundAddress)]
    pub fn refund_address(&self) -> Option<String> {
        self.inner.refund_address.clone()
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
            SwapInStatus::AwaitOpeningTxConfirmation => "AwaitOpeningTxConfirmation",
            SwapInStatus::PayInvoice { .. } => "PayInvoice",
            SwapInStatus::AwaitClaimKeyRevealMessage { .. } => "AwaitClaimKeyRevealMessage",
            SwapInStatus::AwaitClaimTxConfirmation { .. } => "AwaitClaimTxConfirmation",
            SwapInStatus::Claimed { .. } => "Claimed",
            SwapInStatus::Canceled { .. } => "Canceled",
        }
        .to_owned()
    }

    /// Opening transaction id, once a deposit has been announced.
    #[wasm_bindgen(getter, js_name = openingTxId)]
    pub fn opening_tx_id(&self) -> Option<String> {
        self.inner.status.outpoint().map(|o| o.txid.to_string())
    }

    /// Opening transaction output index, once a deposit has been announced.
    #[wasm_bindgen(getter, js_name = openingTxOut)]
    pub fn opening_tx_out(&self) -> Option<u32> {
        self.inner.status.outpoint().map(|o| o.vout)
    }

    /// Payment preimage (hex) the server obtained by paying the claim invoice,
    /// available only in `AwaitClaimKeyRevealMessage`. It hashes to
    /// `paymentHash`, so the payment can be checked before revealing the refund
    /// key.
    #[wasm_bindgen(getter)]
    pub fn preimage(&self) -> Option<String> {
        self.inner.status.preimage().map(hex::encode)
    }

    /// Server's revealed claim privkey (hex), available only after `Canceled`,
    /// enabling an immediate cooperative keypath refund.
    #[wasm_bindgen(getter, js_name = claimPrivkey)]
    pub fn claim_privkey(&self) -> Option<String> {
        match &self.inner.status {
            SwapInStatus::Canceled { claim_privkey, .. } => {
                Some(hex::encode(claim_privkey.secret_bytes()))
            }
            _ => None,
        }
    }

    #[wasm_bindgen(getter, js_name = cancelMessage)]
    pub fn cancel_message(&self) -> Option<String> {
        match &self.inner.status {
            SwapInStatus::Canceled { cancel_message, .. } => Some(cancel_message.clone()),
            _ => None,
        }
    }

    /// Machine-readable cancellation reason (e.g. `TxValidationFailed`),
    /// available only after `Canceled`.
    #[wasm_bindgen(getter, js_name = errorCode)]
    pub fn error_code(&self) -> Option<String> {
        match &self.inner.status {
            SwapInStatus::Canceled { error_code, .. } => Some(error_code.to_string()),
            _ => None,
        }
    }

    /// Checks the reported preimage against the swap's payment hash, proving the
    /// server paid the claim invoice. Call this before revealing the refund key.
    #[wasm_bindgen(js_name = verifyPayment)]
    pub fn verify_payment(&self) -> Result<(), JsError> {
        self.inner.verify_payment().map_err(to_js)
    }

    /// Deposit address for this swap.
    pub fn address(&self, ctx: &SwapContext) -> Result<String, JsError> {
        self.inner.address(ctx.inner()).map_err(to_js)
    }

    /// Every output of `txHex` paying the deposit address.
    pub fn outputs(&self, ctx: &SwapContext, tx_hex: &str) -> Result<Vec<SwapUtxo>, JsError> {
        Ok(self
            .inner
            .outputs(ctx.inner(), tx_hex)
            .map_err(to_js)?
            .into_iter()
            .map(SwapUtxo::new)
            .collect())
    }

    /// Value of `utxo` in sats, unblinded on Liquid.
    pub fn value(&self, ctx: &SwapContext, utxo: &SwapUtxo) -> Result<u64, JsError> {
        self.inner.value(ctx.inner(), utxo.inner()).map_err(to_js)
    }

    /// Splits `utxos` into what is worth refunding at `feeRateSatVb` and what is
    /// not. Every one of them comes back in one of the two sets.
    #[wasm_bindgen(js_name = partitionEconomical)]
    pub fn partition_economical(
        &self,
        ctx: &SwapContext,
        utxos: Vec<SwapUtxo>,
        fee_rate_sat_vb: f64,
    ) -> Result<EconomicalSplit, JsError> {
        let (economical, dust) = self
            .inner
            .partition_economical(ctx.inner(), &utxo::to_utxos(&utxos), fee_rate_sat_vb)
            .map_err(to_js)?;
        Ok(EconomicalSplit::new(economical, dust))
    }

    /// Cooperative refund of `utxos`, as signed transaction hex.
    #[wasm_bindgen(js_name = refundByKeypath)]
    pub fn refund_by_keypath(
        &self,
        ctx: &SwapContext,
        utxos: Vec<SwapUtxo>,
        destination: &str,
        fee_rate_sat_vb: f64,
    ) -> Result<String, JsError> {
        self.inner
            .refund_by_keypath(
                ctx.inner(),
                &utxo::to_utxos(&utxos),
                destination,
                fee_rate_sat_vb,
            )
            .map_err(to_js)
    }

    /// Unilateral CSV refund of `utxos`, as signed transaction hex.
    #[wasm_bindgen(js_name = refundByCsv)]
    pub fn refund_by_csv(
        &self,
        ctx: &SwapContext,
        utxos: Vec<SwapUtxo>,
        destination: &str,
        fee_rate_sat_vb: f64,
    ) -> Result<String, JsError> {
        self.inner
            .refund_by_csv(
                ctx.inner(),
                &utxo::to_utxos(&utxos),
                destination,
                fee_rate_sat_vb,
            )
            .map_err(to_js)
    }
}
