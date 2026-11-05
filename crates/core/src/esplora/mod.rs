//! Esplora blockchain clients (behind the `esplora` feature flag).
//!
//! The chain-neutral [`ChainClient`] trait and its shared types live here; the
//! Bitcoin and Liquid clients are implemented in [`bitcoin`] and [`liquid`].
//! All identifiers cross the trait boundary as chain-neutral
//! strings/bytes so the swap engine never handles chain-specific transaction or
//! address types.
//!
//! Both clients ask again when a read is answered 429, 500 or 503, or a broadcast
//! 429: up to 6 times, waiting from 256 ms and doubling, so a rate limit slows a
//! pass rather than failing it. A rate limit turns a broadcast away unread, while
//! a 500 or 503 may be its answer.

use std::time::Duration;

use ::bitcoin::{Amount, Txid};
use async_trait::async_trait;
use esplora_client::r#async::{DefaultSleeper, Sleeper};
use thiserror::Error;

pub mod bitcoin;
pub mod liquid;

pub use self::{
    bitcoin::{BitcoinChainClient, BitcoinEsplora},
    liquid::{LiquidChainClient, LiquidEsplora},
};

/// How long each try of a request may go unanswered before it fails, for a
/// client built with `new`.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Set for `esplora-client` too, not left to its default: a driver over a rate
/// limit relies on it to slow down rather than fail its swaps.
const MAX_RETRIES: usize = 6;

/// The wait `esplora-client` starts from, which it does not let us set.
const BASE_BACKOFF: Duration = Duration::from_millis(256);

/// Runs `attempt` until `retry` accepts its result, at most [`MAX_RETRIES`] more
/// times, doubling the wait each time.
async fn retrying<T, Fut>(attempt: impl Fn() -> Fut, retry: impl Fn(&T) -> bool) -> T
where
    Fut: Future<Output = T>,
{
    let mut delay = BASE_BACKOFF;
    for _ in 0..MAX_RETRIES {
        let result = attempt().await;
        if !retry(&result) {
            return result;
        }
        drop(result);
        DefaultSleeper::sleep(delay).await;
        delay *= 2;
    }
    attempt().await
}

#[derive(Debug, Error)]
pub enum EsploraError {
    #[error(transparent)]
    Client(#[from] esplora_client::Error),

    #[error(transparent)]
    Http(#[from] reqwest::Error),

    #[error("invalid fee rate: {0} sat/vB")]
    InvalidFeeRate(f64),

    #[error("no fee estimate within {0} blocks")]
    NoFeeEstimate(usize),

    #[error("invalid Esplora response: {0}")]
    InvalidResponse(String),
}

/// A spend as the explorer reports it, only when it names the spending txid
/// and input; anything less reads as unspent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outspend {
    pub txid: Txid,
    pub vin: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EsploraUtxo {
    pub txid: Txid,
    pub vout: u32,
    pub value: Amount,
}

#[derive(Debug, Error)]
pub enum ChainClientError {
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<crate::transaction::TxError> for ChainClientError {
    fn from(e: crate::transaction::TxError) -> Self {
        Self::Other(e.into())
    }
}

/// A single unspent output at an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Utxo {
    pub txid: String,
    pub vout: u32,
}

/// An unspent output as the explorer reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressUtxo {
    pub txid: String,
    pub vout: u32,
    /// Absent when the amount is not public, i.e. a Liquid confidential output.
    /// Reading one takes the blinding key, which only the swap side holds.
    pub value: Option<u64>,
}

/// A spend of a watched outpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutpointSpend {
    /// No spend reported: the output is unspent or unknown to the explorer.
    Unspent,
    /// The output is spent; details of the spending transaction.
    Spent(SpendingTx),
}

/// The transaction spending a watched outpoint. What it means is left to
/// [`crate::transaction::utils`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendingTx {
    pub txid: String,
    /// Input index spending the watched outpoint.
    pub vin: u32,
    /// 0 while the spend sits unconfirmed in the mempool.
    pub confirmations: u32,
    /// Raw spending tx.
    pub tx_hex: String,
}

/// Blockchain query and broadcast surface for one chain.
#[async_trait]
pub trait ChainClient: Send + Sync {
    /// Fee rate for `target`-block confirmation, in sat/vB. Fractional, because
    /// Liquid's relay minimum is 0.1 sat/vB and its estimates never rise above
    /// it. Each chain applies its own floor.
    async fn get_fee_rate(&self, target: usize) -> Result<f64, ChainClientError>;

    /// Confirmation depth of `txid`, or `None` if unknown to the backend.
    async fn get_tx_confirmations(&self, txid: &str) -> Result<Option<u32>, ChainClientError>;

    /// Raw transaction hex for `txid`, if the backend has it.
    async fn get_tx(&self, txid: &str) -> Result<Option<String>, ChainClientError>;

    /// Broadcasts a signed tx (hex); returns its txid.
    async fn broadcast(&self, tx_hex: &str) -> Result<String, ChainClientError>;

    /// Current chain tip height.
    async fn get_height(&self) -> Result<u32, ChainClientError>;

    /// Height of the block confirming `txid`, or `None` if unconfirmed.
    async fn get_confirmation_height(&self, txid: &str) -> Result<Option<u32>, ChainClientError>;

    /// Unspent outputs at `address`.
    async fn address_utxos(&self, address: &str) -> Result<Vec<AddressUtxo>, ChainClientError>;

    /// Inspects the output at `txid:vout` for any spend.
    async fn outpoint_spend(
        &self,
        txid: &str,
        vout: u32,
    ) -> Result<OutpointSpend, ChainClientError>;
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::time::Instant;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

    use super::*;

    pub(super) async fn answering(route: &str, statuses: &[u16]) -> MockServer {
        let server = MockServer::start().await;
        let mock = |status| {
            Mock::given(path(route))
                .respond_with(ResponseTemplate::new(status).set_body_string("7"))
                .expect(1)
        };
        let (last, first) = statuses.split_last().unwrap();
        for status in first {
            mock(*status).up_to_n_times(1).mount(&server).await;
        }
        mock(*last).mount(&server).await;
        server
    }

    pub(super) async fn serving(route: &str, body: serde_json::Value) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test(start_paused = true)]
    async fn retrying_doubles_the_wait_and_gives_up_after_six_retries() {
        let attempts = AtomicUsize::new(0);
        let started = Instant::now();

        let result = retrying(
            || async { attempts.fetch_add(1, Ordering::SeqCst) + 1 },
            |_| true,
        )
        .await;

        assert_eq!(result, 7);
        let elapsed = started.elapsed().as_millis();
        assert!((16_128..16_140).contains(&elapsed), "{elapsed} ms");
    }

    #[tokio::test(start_paused = true)]
    async fn retrying_stops_at_the_first_accepted_result() {
        let attempts = AtomicUsize::new(0);
        let started = Instant::now();

        let result = retrying(
            || async { attempts.fetch_add(1, Ordering::SeqCst) + 1 },
            |attempt| *attempt < 3,
        )
        .await;

        assert_eq!(result, 3);
        let elapsed = started.elapsed().as_millis();
        assert!((768..780).contains(&elapsed), "{elapsed} ms");
    }
}
