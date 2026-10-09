//! The driver's traits implemented by JavaScript objects. The page brings its
//! own server client, chain client, store and signals, and the loop runs in
//! Rust over them. `interfaces.ts` is their TypeScript side.

use anyswap_sdk::{
    api::{
        CancelRequest, CreateSwapRequest, InfoResponse, RevealClaimRequest, RevealPreimageRequest,
        SwapListQuery, SwapListResponse, SwapState,
    },
    client::ClientError,
    driver::{
        ChainClient, ChainError, Destinations, Height, HistoryTx, ServerClient, Signals,
        StoreError, SwapRecord, SwapStore,
    },
    types::Chain,
};
use async_trait::async_trait;
use js_sys::{Promise, Reflect};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// What the page implements and what crosses as a plain object, as
/// TypeScript sees it.
#[wasm_bindgen(typescript_custom_section)]
const INTERFACES: &str = include_str!("interfaces.ts");

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "ServerClient")]
    pub type JsServerClient;
    #[wasm_bindgen(method, catch, js_name = getInfo)]
    fn js_get_info(this: &JsServerClient) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch, js_name = createSwap)]
    fn js_create_swap(this: &JsServerClient, body: JsValue) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch, js_name = getSwap)]
    fn js_get_swap(this: &JsServerClient, swap_id: &str) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch, js_name = listSwaps)]
    fn js_list_swaps(this: &JsServerClient, query: JsValue) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch, js_name = cancel)]
    fn js_cancel(this: &JsServerClient, swap_id: &str, body: JsValue) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch, js_name = revealClaim)]
    fn js_reveal_claim(
        this: &JsServerClient,
        swap_id: &str,
        body: JsValue,
    ) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch, js_name = revealPreimage)]
    fn js_reveal_preimage(
        this: &JsServerClient,
        swap_id: &str,
        body: JsValue,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(typescript_type = "ChainClient")]
    pub type JsChainClient;
    #[wasm_bindgen(method, catch, js_name = getHeight)]
    fn js_get_height(this: &JsChainClient) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch, js_name = getFeeRate)]
    fn js_get_fee_rate(this: &JsChainClient, target: u32) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch, js_name = addressHistory)]
    fn js_address_history(this: &JsChainClient, address: &str) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch, js_name = outspend)]
    fn js_outspend(this: &JsChainClient, txid: &str, vout: u32) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch, js_name = broadcast)]
    fn js_broadcast(this: &JsChainClient, tx_hex: &str) -> Result<Promise, JsValue>;

    #[wasm_bindgen(typescript_type = "SwapStore")]
    pub type JsSwapStore;
    #[wasm_bindgen(method, catch, js_name = save)]
    fn js_save(this: &JsSwapStore, record: JsValue) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch, js_name = load)]
    fn js_load(this: &JsSwapStore, swap_id: &str) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch, js_name = list)]
    fn js_list(this: &JsSwapStore) -> Result<Promise, JsValue>;

    #[wasm_bindgen(typescript_type = "Signals")]
    pub type JsSignals;
    #[wasm_bindgen(method, catch, js_name = next)]
    fn js_next(this: &JsSignals) -> Result<Promise, JsValue>;

    #[wasm_bindgen(typescript_type = "Destinations")]
    pub type JsDestinations;
    #[wasm_bindgen(method, catch, js_name = claimAddress)]
    fn js_claim_address(this: &JsDestinations, chain: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(method, catch, js_name = refundAddress)]
    fn js_refund_address(this: &JsDestinations, chain: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(typescript_type = "Quote")]
    pub type JsQuote;

    #[wasm_bindgen(typescript_type = "TargetAmount")]
    pub type JsTargetAmount;
}

/// The thrown value's message: an `Error`'s own, else its string form.
pub(super) fn message(e: &JsValue) -> String {
    if let Some(error) = e.dyn_ref::<js_sys::Error>() {
        return String::from(error.message());
    }
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}

/// Awaits a call that may have thrown before returning its promise.
async fn call(promise: Result<Promise, JsValue>) -> Result<JsValue, JsValue> {
    JsFuture::from(promise?).await
}

/// A string returned directly or through a promise.
async fn string_or_promise(value: Result<JsValue, JsValue>) -> Result<String, JsValue> {
    let value = match value?.dyn_into::<Promise>() {
        Ok(promise) => JsFuture::from(promise).await?,
        Err(value) => value,
    };
    value
        .as_string()
        .ok_or_else(|| JsValue::from_str("not a string"))
}

/// The value in its JSON shape, `null` for a `None`: amounts in sats as
/// decimal strings, every integer a number.
pub(super) fn to_js<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(JsValue::from)
}

/// An integer may come as a number or a `BigInt`; an amount comes as a string.
pub(super) fn from_js<T: DeserializeOwned>(value: JsValue) -> Result<T, JsValue> {
    serde_wasm_bindgen::from_value(value).map_err(JsValue::from)
}

/// A throw with a numeric `status` is the server's answer, with `code` its
/// error code; anything else is transport.
fn client_error(e: JsValue) -> ClientError {
    let get = |name: &str| Reflect::get(&e, &name.into()).ok();
    match get("status").and_then(|status| status.as_f64()) {
        Some(status) => ClientError::Api {
            status: status as u16,
            message: message(&e),
            code: get("code")
                .and_then(|code| code.as_string())
                .and_then(|code| code.parse().ok()),
        },
        None => ClientError::Transport(message(&e)),
    }
}

fn chain_error(e: JsValue) -> ChainError {
    ChainError(message(&e))
}

fn store_error(e: JsValue) -> StoreError {
    StoreError(message(&e))
}

#[async_trait(?Send)]
impl ServerClient for JsServerClient {
    async fn get_info(&self) -> Result<InfoResponse, ClientError> {
        let value = call(self.js_get_info()).await.map_err(client_error)?;
        from_js(value).map_err(client_error)
    }

    async fn create_swap(&self, req: &CreateSwapRequest) -> Result<(), ClientError> {
        let body = to_js(req).map_err(client_error)?;
        call(self.js_create_swap(body))
            .await
            .map_err(client_error)?;
        Ok(())
    }

    async fn get_swap(&self, swap_id: Uuid) -> Result<SwapState, ClientError> {
        let value = call(self.js_get_swap(&swap_id.to_string()))
            .await
            .map_err(client_error)?;
        from_js(value).map_err(client_error)
    }

    async fn list_swaps(&self, query: &SwapListQuery) -> Result<SwapListResponse, ClientError> {
        let query = to_js(query).map_err(client_error)?;
        let value = call(self.js_list_swaps(query))
            .await
            .map_err(client_error)?;
        from_js(value).map_err(client_error)
    }

    async fn cancel(&self, swap_id: Uuid, req: &CancelRequest) -> Result<(), ClientError> {
        let body = to_js(req).map_err(client_error)?;
        call(self.js_cancel(&swap_id.to_string(), body))
            .await
            .map_err(client_error)?;
        Ok(())
    }

    async fn reveal_claim(
        &self,
        swap_id: Uuid,
        req: &RevealClaimRequest,
    ) -> Result<(), ClientError> {
        let body = to_js(req).map_err(client_error)?;
        call(self.js_reveal_claim(&swap_id.to_string(), body))
            .await
            .map_err(client_error)?;
        Ok(())
    }

    async fn reveal_preimage(
        &self,
        swap_id: Uuid,
        req: &RevealPreimageRequest,
    ) -> Result<(), ClientError> {
        let body = to_js(req).map_err(client_error)?;
        call(self.js_reveal_preimage(&swap_id.to_string(), body))
            .await
            .map_err(client_error)?;
        Ok(())
    }
}

#[derive(Deserialize)]
struct JsHistoryTx {
    hex: String,
    #[serde(default)]
    height: Option<Height>,
}

#[async_trait(?Send)]
impl ChainClient for JsChainClient {
    async fn height(&self) -> Result<Height, ChainError> {
        let value = call(self.js_get_height()).await.map_err(chain_error)?;
        value
            .as_f64()
            .map(|height| height as Height)
            .ok_or_else(|| ChainError("height is not a number".into()))
    }

    async fn fee_rate(&self, target: u32) -> Result<f64, ChainError> {
        let value = call(self.js_get_fee_rate(target))
            .await
            .map_err(chain_error)?;
        value
            .as_f64()
            .ok_or_else(|| ChainError("fee rate is not a number".into()))
    }

    async fn address_history(&self, address: &str) -> Result<Vec<HistoryTx>, ChainError> {
        let value = call(self.js_address_history(address))
            .await
            .map_err(chain_error)?;
        let history: Vec<JsHistoryTx> = from_js(value).map_err(chain_error)?;
        Ok(history
            .into_iter()
            .map(|tx| HistoryTx {
                hex: tx.hex,
                height: tx.height,
            })
            .collect())
    }

    async fn outspend(&self, txid: &str, vout: u32) -> Result<Option<HistoryTx>, ChainError> {
        let value = call(self.js_outspend(txid, vout))
            .await
            .map_err(chain_error)?;
        let spend: Option<JsHistoryTx> = from_js(value).map_err(chain_error)?;
        Ok(spend.map(|tx| HistoryTx {
            hex: tx.hex,
            height: tx.height,
        }))
    }

    async fn broadcast(&self, tx_hex: &str) -> Result<String, ChainError> {
        let value = call(self.js_broadcast(tx_hex)).await.map_err(chain_error)?;
        value
            .as_string()
            .ok_or_else(|| ChainError("txid is not a string".into()))
    }
}

#[async_trait(?Send)]
impl SwapStore for JsSwapStore {
    async fn save(&self, record: &SwapRecord) -> Result<(), StoreError> {
        let value = to_js(record).map_err(store_error)?;
        call(self.js_save(value)).await.map_err(store_error)?;
        Ok(())
    }

    async fn load(&self, swap_id: &Uuid) -> Result<Option<SwapRecord>, StoreError> {
        let value = call(self.js_load(&swap_id.to_string()))
            .await
            .map_err(store_error)?;
        if value.is_undefined() || value.is_null() {
            return Ok(None);
        }
        from_js(value).map(Some).map_err(store_error)
    }

    async fn list(&self) -> Result<Vec<Uuid>, StoreError> {
        let value = call(self.js_list()).await.map_err(store_error)?;
        from_js(value).map_err(store_error)
    }
}

#[async_trait(?Send)]
impl Signals for JsSignals {
    async fn next(&mut self) -> Option<Uuid> {
        let value = call(self.js_next()).await.ok()?;
        value.as_string()?.parse().ok()
    }
}

#[async_trait(?Send)]
impl Destinations for JsDestinations {
    async fn claim_address(&self, chain: Chain) -> Result<String, String> {
        string_or_promise(self.js_claim_address(<&str>::from(chain)))
            .await
            .map_err(|e| message(&e))
    }

    async fn refund_address(&self, chain: Chain) -> Result<String, String> {
        string_or_promise(self.js_refund_address(<&str>::from(chain)))
            .await
            .map_err(|e| message(&e))
    }
}
