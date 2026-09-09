//! Esplora blockchain clients (behind the `esplora` feature flag).
//!
//! The chain-neutral [`ChainClient`] trait and its shared types live here; the
//! Bitcoin and Liquid clients are implemented in [`bitcoin`] and [`liquid`].
//! All identifiers cross the trait boundary as chain-neutral
//! strings/bytes so the swap engine never handles chain-specific transaction or
//! address types.

use ::bitcoin::{Amount, Txid};
use async_trait::async_trait;
use thiserror::Error;

pub mod bitcoin;
pub mod liquid;

pub use self::{
    bitcoin::{BitcoinChainClient, BitcoinEsplora},
    liquid::{LiquidChainClient, LiquidEsplora},
};

#[derive(Debug, Error)]
pub enum EsploraError {
    #[error(transparent)]
    Client(#[from] esplora_client::Error),

    #[error(transparent)]
    Http(#[from] reqwest::Error),

    #[error("invalid fee rate: {0} sat/vB")]
    InvalidFeeRate(f64),

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
