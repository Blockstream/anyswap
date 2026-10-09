//! The driver's knobs as the page sets them: the SDK's `Config` and
//! `ChainConfig` behind constructors giving their defaults and a property per
//! field, durations in seconds as `bigint`.

use std::time::Duration;

use anyswap_sdk::driver::{ChainConfig as SdkChainConfig, Config as SdkConfig};
use wasm_bindgen::prelude::*;

/// Knobs true of the driver whatever it is swapping, given to
/// `Driver.withConfig`. `new Config()` is the default; set the properties to
/// change. Wall-clock deadlines are counted on the driver's own clock, never
/// from a time the server reports.
#[wasm_bindgen]
#[derive(Clone, Default)]
pub struct Config {
    inner: SdkConfig,
}

#[wasm_bindgen]
impl Config {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Config {
        Config::default()
    }

    /// Seconds from `started_at` until a swap with nothing on chain closes.
    #[wasm_bindgen(getter, js_name = fundingTimeout)]
    pub fn funding_timeout(&self) -> u64 {
        self.inner.funding_timeout.as_secs()
    }

    #[wasm_bindgen(setter, js_name = fundingTimeout)]
    pub fn set_funding_timeout(&mut self, secs: u64) {
        self.inner.funding_timeout = Duration::from_secs(secs);
    }

    /// The longest a loop waits between passes without a signal, in seconds.
    #[wasm_bindgen(getter, js_name = pollInterval)]
    pub fn poll_interval(&self) -> u64 {
        self.inner.poll_interval.as_secs()
    }

    #[wasm_bindgen(setter, js_name = pollInterval)]
    pub fn set_poll_interval(&mut self, secs: u64) {
        self.inner.poll_interval = Duration::from_secs(secs);
    }

    /// The target every fee estimate is read at, in blocks.
    #[wasm_bindgen(getter, js_name = feeConfTarget)]
    pub fn fee_conf_target(&self) -> u32 {
        self.inner.fee_conf_target
    }

    #[wasm_bindgen(setter, js_name = feeConfTarget)]
    pub fn set_fee_conf_target(&mut self, blocks: u32) {
        self.inner.fee_conf_target = blocks;
    }
    /// Seconds from a swap-in's end that it stays in `run`'s set once nothing
    /// of ours under its script is left to move.
    #[wasm_bindgen(getter, js_name = refundTimeout)]
    pub fn refund_timeout(&self) -> u64 {
        self.inner.refund_timeout.as_secs()
    }

    #[wasm_bindgen(setter, js_name = refundTimeout)]
    pub fn set_refund_timeout(&mut self, secs: u64) {
        self.inner.refund_timeout = Duration::from_secs(secs);
    }
}

impl Config {
    /// Read by the driver, which only builds for wasm32.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn inner(&self) -> &SdkConfig {
        &self.inner
    }
}

/// Knobs that only mean anything per chain, given with the chain to
/// `Driver.withChain`. `ChainConfig.bitcoin()` and `ChainConfig.liquid()` are
/// the defaults; set the properties to change. Block deadlines count from a
/// confirmation height, since a timelock on an unconfirmed output has not
/// started.
#[wasm_bindgen]
#[derive(Clone)]
pub struct ChainConfig {
    inner: SdkChainConfig,
}

#[wasm_bindgen]
impl ChainConfig {
    /// The Bitcoin defaults.
    pub fn bitcoin() -> ChainConfig {
        ChainConfig {
            inner: SdkChainConfig::bitcoin(),
        }
    }

    /// The Liquid defaults.
    pub fn liquid() -> ChainConfig {
        ChainConfig {
            inner: SdkChainConfig::liquid(),
        }
    }

    /// The depth that opens the swap-out release gate, and the depth a spend
    /// reaches before it closes a swap, in blocks.
    #[wasm_bindgen(getter, js_name = minConfs)]
    pub fn min_confs(&self) -> u32 {
        self.inner.min_confs
    }

    #[wasm_bindgen(setter, js_name = minConfs)]
    pub fn set_min_confs(&mut self, blocks: u32) {
        self.inner.min_confs = blocks;
    }

    /// How much CSV must remain when the preimage goes out, in blocks.
    #[wasm_bindgen(getter, js_name = claimMargin)]
    pub fn claim_margin(&self) -> u32 {
        self.inner.claim_margin
    }

    #[wasm_bindgen(setter, js_name = claimMargin)]
    pub fn set_claim_margin(&mut self, blocks: u32) {
        self.inner.claim_margin = blocks;
    }

    /// The longest CSV accepted on a deposit of our own, in blocks.
    #[wasm_bindgen(getter, js_name = maxRefundCsv)]
    pub fn max_refund_csv(&self) -> u32 {
        self.inner.max_refund_csv
    }

    #[wasm_bindgen(setter, js_name = maxRefundCsv)]
    pub fn set_max_refund_csv(&mut self, blocks: u32) {
        self.inner.max_refund_csv = blocks;
    }

    /// How far past `dest_csv` a hold invoice may hold the user's payment,
    /// in blocks.
    #[wasm_bindgen(getter, js_name = maxHtlcHold)]
    pub fn max_htlc_hold(&self) -> u32 {
        self.inner.max_htlc_hold
    }

    #[wasm_bindgen(setter, js_name = maxHtlcHold)]
    pub fn set_max_htlc_hold(&mut self, blocks: u32) {
        self.inner.max_htlc_hold = blocks;
    }

    /// The rate no spend goes above, in sat/vB.
    #[wasm_bindgen(getter, js_name = maxFeeRate)]
    pub fn max_fee_rate(&self) -> f64 {
        self.inner.max_fee_rate
    }

    #[wasm_bindgen(setter, js_name = maxFeeRate)]
    pub fn set_max_fee_rate(&mut self, sat_per_vb: f64) {
        self.inner.max_fee_rate = sat_per_vb;
    }

    /// Expected block interval in seconds, to compare a block deadline
    /// against a wall-clock one.
    #[wasm_bindgen(getter, js_name = blockTime)]
    pub fn block_time(&self) -> u64 {
        self.inner.block_time
    }

    #[wasm_bindgen(setter, js_name = blockTime)]
    pub fn set_block_time(&mut self, secs: u64) {
        self.inner.block_time = secs;
    }
}

impl ChainConfig {
    /// Read by the driver, which only builds for wasm32.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn inner(&self) -> &SdkChainConfig {
        &self.inner
    }
}
