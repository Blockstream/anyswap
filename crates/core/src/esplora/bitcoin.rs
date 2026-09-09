//! Esplora Bitcoin blockchain client for fetching on-chain data needed during
//! swap execution, plus its [`ChainClient`] implementation.
//!
//! [`BitcoinEsplora`] derefs to the underlying [`AsyncClient`] for direct access
//! to all esplora API methods; [`BitcoinChainClient`] wraps it to satisfy the
//! chain-neutral [`ChainClient`] trait.

use std::str::FromStr;

use async_trait::async_trait;
use bitcoin::{Address, Transaction, Txid};
use esplora_client::r#async::AsyncClient;

use super::{
    AddressUtxo, ChainClient, ChainClientError, EsploraError, EsploraUtxo, OutpointSpend, Outspend,
    SpendingTx,
};
use crate::transaction::{decode_tx_hex, encode_tx_hex};

/// Relay-minimum floor applied to fee estimates: Bitcoin's default
/// `minrelaytxfee` is one sat/vB, so nothing below it propagates.
const MIN_FEE_RATE_SAT_PER_VB: f64 = 1.0;

/// Confirmed transactions per page of the address history endpoint.
pub(crate) const HISTORY_PAGE: usize = 25;

#[derive(Debug, Clone)]
pub struct BitcoinEsplora(AsyncClient);

impl std::ops::Deref for BitcoinEsplora {
    type Target = AsyncClient;

    fn deref(&self) -> &AsyncClient {
        &self.0
    }
}

impl BitcoinEsplora {
    pub fn new(url: &str) -> Result<Self, EsploraError> {
        let client = esplora_client::Builder::new(url).build_async()?;
        Ok(Self(client))
    }

    pub async fn get_fee_rate(&self, target: usize) -> Result<f64, EsploraError> {
        let estimates = self.0.get_fee_estimates().await?;
        // Fall back to the relay minimum when no estimates are available (e.g. regtest).
        let estimate = esplora_client::convert_fee_rate(target, estimates)
            .map_or(MIN_FEE_RATE_SAT_PER_VB, f64::from);

        if !estimate.is_finite() || estimate < 0.0 {
            return Err(EsploraError::InvalidFeeRate(estimate));
        }
        Ok(estimate.max(MIN_FEE_RATE_SAT_PER_VB))
    }

    pub async fn get_tx_confirmations(&self, txid: &Txid) -> Result<Option<u32>, EsploraError> {
        let status = match self.0.get_tx_status(txid).await {
            Ok(status) => status,
            Err(esplora_client::Error::HttpResponse { status: 404, .. }) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let block_height = match status.block_height {
            Some(h) if status.confirmed => h,
            _ => return Ok(Some(0)),
        };
        let tip = self.0.get_height().await?;
        Ok(Some(tip.saturating_sub(block_height) + 1))
    }

    pub async fn get_tx(&self, txid: &Txid) -> Result<Option<Transaction>, EsploraError> {
        Ok(self.0.get_tx(txid).await?)
    }

    pub async fn broadcast(&self, tx: &Transaction) -> Result<Txid, EsploraError> {
        let txid = tx.compute_txid();
        self.0.broadcast(tx).await?;
        Ok(txid)
    }

    pub async fn get_height(&self) -> Result<u32, EsploraError> {
        Ok(self.0.get_height().await?)
    }

    pub async fn get_outspend(
        &self,
        txid: &Txid,
        vout: u32,
    ) -> Result<Option<Outspend>, EsploraError> {
        let Some(status) = self.0.get_output_status(txid, vout as u64).await? else {
            return Ok(None);
        };
        Ok(match (status.spent, status.txid, status.vin) {
            (true, Some(txid), Some(vin)) => Some(Outspend {
                txid,
                vin: vin as u32,
            }),
            _ => None,
        })
    }

    pub async fn get_confirmation_height(&self, txid: &Txid) -> Result<Option<u32>, EsploraError> {
        let status = self.0.get_tx_status(txid).await?;
        Ok(status.block_height.filter(|_| status.confirmed))
    }

    /// The address's full current UTXO set (confirmed and mempool). Unlike the
    /// tx-history endpoint there is no newest-N page cap, so a deposit can never
    /// be hidden behind unrelated transactions to the same address.
    pub async fn address_utxos(&self, address: &Address) -> Result<Vec<EsploraUtxo>, EsploraError> {
        let utxos = self.0.get_address_utxos(address).await?;
        Ok(utxos
            .into_iter()
            .map(|u| EsploraUtxo {
                txid: u.txid,
                vout: u.vout,
                value: u.value,
            })
            .collect())
    }

    /// Every transaction touching `address` with its confirmation height,
    /// newest first, paged to the end.
    pub async fn address_txs(
        &self,
        address: &Address,
    ) -> Result<Vec<(Transaction, Option<u32>)>, EsploraError> {
        let mut txs = Vec::new();
        let mut last_seen = None;
        loop {
            let page = self.0.get_address_txs(address, last_seen).await?;
            let confirmed = page.iter().filter(|tx| tx.status.confirmed).count();
            last_seen = page
                .iter()
                .rev()
                .find(|tx| tx.status.confirmed)
                .map(|tx| tx.txid);
            txs.extend(page.iter().map(|tx| {
                let height = tx.status.block_height.filter(|_| tx.status.confirmed);
                (tx.to_tx(), height)
            }));
            if confirmed < HISTORY_PAGE {
                return Ok(txs);
            }
        }
    }
}

impl From<EsploraError> for ChainClientError {
    fn from(e: EsploraError) -> Self {
        ChainClientError::Other(e.into())
    }
}

/// [`ChainClient`] implementation for Bitcoin, over the esplora client.
#[derive(Debug, Clone)]
pub struct BitcoinChainClient(BitcoinEsplora);

impl BitcoinChainClient {
    pub fn new(url: &str) -> Result<Self, EsploraError> {
        Ok(Self(BitcoinEsplora::new(url)?))
    }
}

#[async_trait]
impl ChainClient for BitcoinChainClient {
    async fn get_fee_rate(&self, target: usize) -> Result<f64, ChainClientError> {
        Ok(self.0.get_fee_rate(target).await?)
    }

    async fn get_tx_confirmations(&self, txid: &str) -> Result<Option<u32>, ChainClientError> {
        Ok(self.0.get_tx_confirmations(&parse_txid(txid)?).await?)
    }

    async fn get_tx(&self, txid: &str) -> Result<Option<String>, ChainClientError> {
        Ok(self
            .0
            .get_tx(&parse_txid(txid)?)
            .await?
            .map(|tx| encode_tx_hex(&tx)))
    }

    async fn broadcast(&self, tx_hex: &str) -> Result<String, ChainClientError> {
        let tx = decode_tx_hex(tx_hex)?;
        Ok(self.0.broadcast(&tx).await?.to_string())
    }

    async fn get_height(&self) -> Result<u32, ChainClientError> {
        Ok(self.0.get_height().await?)
    }

    async fn get_confirmation_height(&self, txid: &str) -> Result<Option<u32>, ChainClientError> {
        Ok(self.0.get_confirmation_height(&parse_txid(txid)?).await?)
    }

    async fn address_utxos(&self, address: &str) -> Result<Vec<AddressUtxo>, ChainClientError> {
        let utxos = self.0.address_utxos(&parse_address(address)?).await?;
        Ok(utxos
            .into_iter()
            .map(|utxo| AddressUtxo {
                txid: utxo.txid.to_string(),
                vout: utxo.vout,
                value: Some(utxo.value.to_sat()),
            })
            .collect())
    }

    async fn outpoint_spend(
        &self,
        txid: &str,
        vout: u32,
    ) -> Result<OutpointSpend, ChainClientError> {
        let Some(outspend) = self.0.get_outspend(&parse_txid(txid)?, vout).await? else {
            return Ok(OutpointSpend::Unspent);
        };
        let (spending_txid, vin) = (outspend.txid, outspend.vin);
        let confirmations = self
            .0
            .get_tx_confirmations(&spending_txid)
            .await?
            .unwrap_or(0);
        // The backend itself reported this txid as the spender, so a missing
        // body is an inconsistent backend.
        let Some(tx) = self.0.get_tx(&spending_txid).await? else {
            return Err(ChainClientError::Other(anyhow::anyhow!(
                "spending tx {spending_txid} reported by outspend but not found"
            )));
        };
        let tx_hex = encode_tx_hex(&tx);
        Ok(OutpointSpend::Spent(SpendingTx {
            txid: spending_txid.to_string(),
            vin,
            confirmations,
            tx_hex,
        }))
    }
}

fn parse_txid(txid: &str) -> Result<Txid, ChainClientError> {
    Txid::from_str(txid)
        .map_err(|e| ChainClientError::Other(anyhow::anyhow!("invalid txid {txid}: {e}")))
}

fn parse_address(address: &str) -> Result<Address, ChainClientError> {
    Ok(Address::from_str(address)
        .map_err(|e| ChainClientError::Other(anyhow::anyhow!("invalid address {address}: {e}")))?
        .assume_checked())
}
