//! The driver in the browser: the Rust SDK's loop, unchanged, over a server
//! client, a chain client, a store and a signal channel the page implements
//! in JavaScript. Quotes, statuses and records cross as the JSON form of the
//! Rust types, with amounts in sats as decimal strings and every integer a
//! number.

mod error;
mod interfaces;
mod types;

use std::str::FromStr;

use anyswap_sdk::{
    driver::{
        Cancellation, Driver as SdkDriver, Handle as SdkHandle, Quote as SdkQuote, TargetAmount,
        Updates as SdkUpdates, spawn,
    },
    types::Chain,
    wallet::Wallet as SdkWallet,
};
use serde::Serialize;
use wasm_bindgen::prelude::*;

use self::error::invalid;
pub use self::{
    error::{DriverError, DriverErrorCode},
    interfaces::{
        JsChainClient, JsDestinations, JsQuote, JsServerClient, JsSignals, JsSwapStore,
        JsTargetAmount,
    },
    types::{RestoreFailure, RestoreReport, RunState},
};
use crate::{
    config::{ChainConfig, Config},
    context::SwapContext,
};

fn parse_chain(chain: &str) -> Result<Chain, DriverError> {
    Chain::from_str(chain).map_err(|_| {
        invalid(format!(
            "unknown chain {chain:?}; expected bitcoin or liquid"
        ))
    })
}

fn parse_uuid(swap_id: &str) -> Result<uuid::Uuid, DriverError> {
    anyswap_sdk::codec::parse_uuid(swap_id).map_err(invalid)
}

fn to_js<T: Serialize>(value: &T) -> Result<JsValue, DriverError> {
    interfaces::to_js(value).map_err(|e| invalid(interfaces::message(&e)))
}

fn quote_from_js(quote: JsQuote) -> Result<SdkQuote, DriverError> {
    interfaces::from_js(quote.into())
        .map_err(|e| invalid(format!("invalid quote: {}", interfaces::message(&e))))
}

/// Drives every swap in its store. `run` drives them all in the background;
/// a `SwapHandle` drives one by hand.
#[wasm_bindgen]
pub struct Driver {
    inner: SdkDriver<SdkWallet>,
}

#[wasm_bindgen]
impl Driver {
    #[wasm_bindgen(constructor)]
    pub fn new(ctx: &SwapContext, server: JsServerClient, store: JsSwapStore) -> Driver {
        Driver {
            inner: SdkDriver::new(ctx.inner().clone(), server, store),
        }
    }

    /// Registers `bitcoin` or `liquid` with its client under `config`, or
    /// the chain's default policy without one; a chain without a client
    /// cannot be swapped on. The `config` is taken over: build one per call.
    #[wasm_bindgen(js_name = withChain)]
    pub fn with_chain(
        self,
        chain: &str,
        client: JsChainClient,
        config: Option<ChainConfig>,
    ) -> Result<Driver, DriverError> {
        let chain = parse_chain(chain)?;
        let config = config.unwrap_or_else(|| match chain {
            Chain::Bitcoin => ChainConfig::bitcoin(),
            Chain::Liquid => ChainConfig::liquid(),
        });
        Ok(Driver {
            inner: self.inner.with_chain(chain, client, *config.inner()),
        })
    }

    /// Sets the driver-wide knobs; without this call they are `new Config()`.
    /// The per-chain policy comes with `withChain`.
    #[wasm_bindgen(js_name = withConfig)]
    pub fn with_config(self, config: &Config) -> Driver {
        Driver {
            inner: self.inner.with_config(config.inner().clone()),
        }
    }

    /// Takes a push channel to pace the loops by; without one they pace on
    /// the poll interval alone.
    #[wasm_bindgen(js_name = withSignals)]
    pub fn with_signals(self, signals: JsSignals) -> Driver {
        Driver {
            inner: self.inner.with_signals(signals),
        }
    }

    /// Takes the hook asked for an address when a swap is created without
    /// one.
    #[wasm_bindgen(js_name = withDestinations)]
    pub fn with_destinations(self, destinations: JsDestinations) -> Driver {
        Driver {
            inner: self.inner.with_destinations(destinations),
        }
    }

    /// Wakes any loop waiting on `swapId`, as a signal from the channel
    /// would.
    pub fn signal(&self, swap_id: &str) -> Result<(), DriverError> {
        self.inner.signal(parse_uuid(swap_id)?);
        Ok(())
    }

    /// A quote from `source` to `dest`, an end being Lightning when its
    /// chain is left out, pinned by `target`.
    pub async fn quote(
        &self,
        source: Option<String>,
        dest: Option<String>,
        target: JsTargetAmount,
    ) -> Result<JsQuote, DriverError> {
        let source = source.as_deref().map(parse_chain).transpose()?;
        let dest = dest.as_deref().map(parse_chain).transpose()?;
        let target: TargetAmount = interfaces::from_js(target.into()).map_err(|e| {
            invalid(format!(
                "invalid target amount: {}",
                interfaces::message(&e)
            ))
        })?;
        let quote = self.inner.quote(source, dest, target).await?;
        Ok(to_js(&quote)?.unchecked_into())
    }

    /// Creates a SwapIn against `quote` with the invoice minted for its
    /// `receive_amount`. The handle carries where and how much to deposit.
    #[wasm_bindgen(js_name = swapIn)]
    pub async fn swap_in(
        &self,
        quote: JsQuote,
        claim_invoice: String,
        refund_address: Option<String>,
    ) -> Result<SwapHandle, DriverError> {
        let quote = quote_from_js(quote)?;
        let swap = self
            .inner
            .swap_in(quote, claim_invoice, refund_address)
            .await?;
        Ok(SwapHandle { inner: swap })
    }

    /// Creates a SwapOut against `quote` paying `claimAddress`, or the
    /// destinations hook's address without one. The handle carries the hold
    /// invoice to pay.
    #[wasm_bindgen(js_name = swapOut)]
    pub async fn swap_out(
        &self,
        quote: JsQuote,
        claim_address: Option<String>,
    ) -> Result<SwapHandle, DriverError> {
        let quote = quote_from_js(quote)?;
        let swap = self.inner.swap_out(quote, claim_address).await?;
        Ok(SwapHandle { inner: swap })
    }

    /// The stored swap as a fresh handle, or `undefined`.
    pub async fn get(&self, swap_id: String) -> Result<Option<SwapHandle>, DriverError> {
        let swap = self.inner.get(parse_uuid(&swap_id)?).await?;
        Ok(swap.map(|inner| SwapHandle { inner }))
    }

    /// Rebuilds one swap from the server's copy of our signed request. It is
    /// `Restoring` until its first pass, which `run` makes, or `step`.
    pub async fn restore(&self, swap_id: String) -> Result<SwapHandle, DriverError> {
        let inner = self.inner.restore(parse_uuid(&swap_id)?).await?;
        Ok(SwapHandle { inner })
    }

    /// Rebuilds every swap the server lists for this wallet that the store
    /// lacks, as `restore` does one. Resolves to the ids restored and, for
    /// each swap that failed a check, which one; a swap the server holds no
    /// signed request for fails as unsigned.
    #[wasm_bindgen(js_name = restoreAll)]
    pub async fn restore_all(&self) -> Result<RestoreReport, DriverError> {
        let report = self.inner.restore_all().await?;
        Ok(report.into())
    }

    /// Drives every swap in the store that is not quiet, in the background,
    /// until `stop`. Each pass re-reads the store; a second call is a no-op.
    pub fn run(&self) {
        spawn(self.inner.run());
    }

    /// Ends `run` after the pass in flight. Final; `waitStopped` is the wait.
    pub fn stop(&self) {
        self.inner.stop();
    }

    /// Resolves once `run` has exited and nothing writes to the store.
    #[wasm_bindgen(js_name = waitStopped)]
    pub async fn wait_stopped(&self) {
        self.inner.wait_stopped().await;
    }

    #[wasm_bindgen(getter)]
    pub fn state(&self) -> RunState {
        self.inner.state().into()
    }

    /// Every swap a pass changed, from `run` and from `step`.
    pub fn subscribe(&self) -> Updates {
        Updates {
            inner: self.inner.subscribe(),
        }
    }
}

/// A swap bound to its driver: its id and type, and the calls that act on
/// it. It keeps no copy of the swap, so every read is the store's or the
/// driver's newest.
#[wasm_bindgen]
pub struct SwapHandle {
    inner: SdkHandle<SdkWallet>,
}

#[wasm_bindgen]
impl SwapHandle {
    #[wasm_bindgen(getter, js_name = swapId)]
    pub fn swap_id(&self) -> String {
        self.inner.swap_id().to_string()
    }

    /// `SwapIn` or `SwapOut`.
    #[wasm_bindgen(getter, js_name = swapType)]
    pub fn swap_type(&self) -> String {
        self.inner.swap_type().to_string()
    }

    /// The stored status, as the last pass left it, in the JSON form of the
    /// Rust `Status`: the variant under `status`, its fields beside it.
    pub async fn status(&self) -> Result<JsValue, DriverError> {
        to_js(&self.inner.status().await?)
    }

    #[wasm_bindgen(js_name = isTerminal)]
    pub async fn is_terminal(&self) -> Result<bool, DriverError> {
        Ok(self.inner.status().await?.is_terminal())
    }

    /// The stored record in its JSON form.
    pub async fn record(&self) -> Result<JsValue, DriverError> {
        to_js(&self.inner.record().await?)
    }

    /// The error of this swap's last failing pass under `run`, `undefined`
    /// after a working one.
    #[wasm_bindgen(getter)]
    pub fn error(&self) -> Option<DriverError> {
        self.inner.error().map(|e| DriverError::from(&e))
    }

    /// SwapIn: the deposit address; the record's `send_amount` goes here.
    pub async fn address(&self) -> Result<String, DriverError> {
        Ok(self.inner.address().await?)
    }

    /// SwapOut: the hold invoice to pay; the record's `send_amount` is its
    /// amount.
    #[wasm_bindgen(js_name = claimInvoice)]
    pub async fn claim_invoice(&self) -> Result<String, DriverError> {
        Ok(self.inner.claim_invoice().await?)
    }

    /// One pass, now. Resolves to the status it left.
    pub async fn step(&self) -> Result<JsValue, DriverError> {
        let status = self.inner.step().await?;
        to_js(&status)
    }

    /// Waits for a signal or the poll interval, whichever comes first, then
    /// makes one pass. Resolves to the status it left.
    pub async fn advance(&self) -> Result<JsValue, DriverError> {
        let status = self.inner.advance().await?;
        to_js(&status)
    }

    /// `Requested`: the swap is canceled, or closes once the server confirms;
    /// `TooLate` means the payment landed first and the swap continues.
    pub async fn cancel(&self) -> Result<String, DriverError> {
        let outcome = match self.inner.cancel().await? {
            Cancellation::Requested => "Requested",
            Cancellation::TooLate => "TooLate",
        };
        Ok(outcome.to_owned())
    }

    /// SwapIn: what under the script is ours, as the last pass left it, in
    /// the JSON form of the Rust `Fund` list.
    pub async fn funds(&self) -> Result<JsValue, DriverError> {
        to_js(&self.inner.funds().await?)
    }

    /// SwapIn: one pass that refunds the `Refundable` funds to
    /// `destination`. The txid, or `undefined` with nothing refundable yet or
    /// an earlier refund still `Refunding`.
    pub async fn refund(&self, destination: String) -> Result<Option<String>, DriverError> {
        Ok(self.inner.refund(&destination).await?)
    }
}

/// Every swap a pass changed, as `run` and `step` produce them, plus a
/// handle for every failing pass under `run`.
#[wasm_bindgen]
pub struct Updates {
    inner: SdkUpdates<SdkWallet>,
}

#[wasm_bindgen]
impl Updates {
    /// The next changed swap, or `undefined` once the driver is stopped or gone.
    pub async fn next(&mut self) -> Option<SwapHandle> {
        self.inner.next().await.map(|inner| SwapHandle { inner })
    }
}
