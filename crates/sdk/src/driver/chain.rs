use async_trait::async_trait;
use thiserror::Error;

use super::{
    record::Height,
    rt::{MaybeSend, MaybeSync},
};

#[derive(Debug, Error)]
#[error("chain: {0}")]
pub struct ChainError(pub String);

/// One transaction of an address's history, mempool included.
#[derive(Clone, Debug)]
pub struct HistoryTx {
    pub hex: String,
    /// The confirming block, `None` while in the mempool.
    pub height: Option<Height>,
}

/// Query and broadcast on one chain. The driver reads a script's whole history
/// and interprets it itself, so an implementation fetches bytes by a fixed
/// recipe and never decides anything.
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait ChainClient: MaybeSend + MaybeSync {
    async fn height(&self) -> Result<Height, ChainError>;

    /// Estimate for confirmation within `target` blocks, in sat/vB.
    async fn fee_rate(&self, target: u32) -> Result<f64, ChainError>;

    /// Every transaction paying or spending `address`.
    async fn address_history(&self, address: &str) -> Result<Vec<HistoryTx>, ChainError>;

    /// Broadcasts a signed transaction and returns its txid.
    async fn broadcast(&self, tx_hex: &str) -> Result<String, ChainError>;
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl<C: ChainClient + ?Sized> ChainClient for std::sync::Arc<C> {
    async fn height(&self) -> Result<Height, ChainError> {
        (**self).height().await
    }

    async fn fee_rate(&self, target: u32) -> Result<f64, ChainError> {
        (**self).fee_rate(target).await
    }

    async fn address_history(&self, address: &str) -> Result<Vec<HistoryTx>, ChainError> {
        (**self).address_history(address).await
    }

    async fn broadcast(&self, tx_hex: &str) -> Result<String, ChainError> {
        (**self).broadcast(tx_hex).await
    }
}

#[cfg(feature = "esplora")]
mod esplora {
    use std::str::FromStr;

    use anyswap_core::{
        esplora::{BitcoinEsplora, EsploraError, LiquidEsplora},
        transaction::bitcoin::{decode_tx_hex, encode_tx_hex},
    };
    use async_trait::async_trait;

    use super::{ChainClient, ChainError, HistoryTx};
    use crate::driver::record::Height;

    impl From<EsploraError> for ChainError {
        fn from(e: EsploraError) -> Self {
            Self(e.to_string())
        }
    }

    fn invalid(what: &str, e: impl std::fmt::Display) -> ChainError {
        ChainError(format!("invalid {what}: {e}"))
    }

    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    impl ChainClient for BitcoinEsplora {
        async fn height(&self) -> Result<Height, ChainError> {
            Ok(BitcoinEsplora::get_height(self).await?)
        }

        async fn fee_rate(&self, target: u32) -> Result<f64, ChainError> {
            Ok(BitcoinEsplora::get_fee_rate(self, target as usize).await?)
        }

        async fn address_history(&self, address: &str) -> Result<Vec<HistoryTx>, ChainError> {
            let address = bitcoin::Address::from_str(address)
                .map_err(|e| invalid("address", e))?
                .assume_checked();
            Ok(self
                .address_txs(&address)
                .await?
                .into_iter()
                .map(|(tx, height)| HistoryTx {
                    hex: encode_tx_hex(&tx),
                    height,
                })
                .collect())
        }

        async fn broadcast(&self, tx_hex: &str) -> Result<String, ChainError> {
            let tx = decode_tx_hex(tx_hex).map_err(|e| invalid("transaction", e))?;
            Ok(BitcoinEsplora::broadcast(self, &tx).await?.to_string())
        }
    }

    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    impl ChainClient for LiquidEsplora {
        async fn height(&self) -> Result<Height, ChainError> {
            Ok(LiquidEsplora::get_height(self).await?)
        }

        async fn fee_rate(&self, target: u32) -> Result<f64, ChainError> {
            Ok(LiquidEsplora::get_fee_rate(self, target as usize).await?)
        }

        async fn address_history(&self, address: &str) -> Result<Vec<HistoryTx>, ChainError> {
            let address =
                elements::Address::from_str(address).map_err(|e| invalid("address", e))?;
            let mut history = Vec::new();
            for (txid, height) in self.address_txs(&address).await? {
                if let Some(tx) = self.get_tx(&txid).await? {
                    history.push(HistoryTx {
                        hex: hex::encode(elements::encode::serialize(&tx)),
                        height,
                    });
                }
            }
            Ok(history)
        }

        async fn broadcast(&self, tx_hex: &str) -> Result<String, ChainError> {
            let tx = anyswap_core::transaction::liquid::decode_tx_hex(tx_hex)
                .map_err(|e| invalid("transaction", e))?;
            Ok(LiquidEsplora::broadcast(self, &tx).await?.to_string())
        }
    }
}
