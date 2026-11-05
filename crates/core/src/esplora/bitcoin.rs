//! Esplora Bitcoin blockchain client for fetching on-chain data needed during
//! swap execution, plus its [`ChainClient`] implementation.
//!
//! [`BitcoinEsplora`] derefs to the underlying [`AsyncClient`] for direct access
//! to all esplora API methods; [`BitcoinChainClient`] wraps it to satisfy the
//! chain-neutral [`ChainClient`] trait.

use std::{collections::HashSet, str::FromStr, time::Duration};

use async_trait::async_trait;
use bitcoin::{Address, Transaction, Txid};
use esplora_client::r#async::AsyncClient;

use super::{
    AddressUtxo, ChainClient, ChainClientError, DEFAULT_TIMEOUT, EsploraError, EsploraUtxo,
    MAX_RETRIES, OutpointSpend, Outspend, SpendingTx, retrying,
};
use crate::{
    fee,
    transaction::{decode_tx_hex, encode_tx_hex},
    types::Chain,
};

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
        Self::with_timeout(url, DEFAULT_TIMEOUT)
    }

    /// A client whose every try of a request fails once it goes unanswered for
    /// `timeout`, counted in whole seconds.
    pub fn with_timeout(url: &str, timeout: Duration) -> Result<Self, EsploraError> {
        let client = esplora_client::Builder::new(url)
            .timeout(timeout.as_secs())
            .max_retries(MAX_RETRIES)
            .build_async()?;
        Ok(Self(client))
    }

    pub async fn get_fee_rate(&self, target: usize) -> Result<f64, EsploraError> {
        let estimates = self.0.get_fee_estimates().await?;
        let estimate = esplora_client::convert_fee_rate(target, estimates)
            .map(f64::from)
            .ok_or(EsploraError::NoFeeEstimate(target))?;

        if !estimate.is_finite() || estimate < 0.0 {
            return Err(EsploraError::InvalidFeeRate(estimate));
        }
        Ok(estimate.max(fee::min_fee_rate(Chain::Bitcoin)))
    }

    pub async fn get_tx_confirmations(&self, txid: &Txid) -> Result<Option<u32>, EsploraError> {
        // `/tx/:txid/status` reports an unknown txid as unconfirmed; `/tx/:txid` 404s.
        let Some(tx) = self.0.get_tx_info(txid).await? else {
            return Ok(None);
        };
        let status = tx.status;
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
        retrying(
            || self.0.broadcast(tx),
            |result| {
                matches!(
                    result,
                    Err(esplora_client::Error::HttpResponse { status: 429, .. })
                )
            },
        )
        .await?;
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

    /// The address's full current UTXO set (confirmed and mempool), each output
    /// once. Unlike the tx-history endpoint there is no newest-N page cap, so a
    /// deposit can never be hidden behind unrelated transactions to the same
    /// address.
    pub async fn address_utxos(&self, address: &Address) -> Result<Vec<EsploraUtxo>, EsploraError> {
        let utxos = self.0.get_address_utxos(address).await?;
        // electrs indexes a block before it updates its mempool, and until then
        // lists a transaction the block confirmed from both.
        let mut seen = HashSet::new();
        Ok(utxos
            .into_iter()
            .filter(|u| seen.insert((u.txid, u.vout)))
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
        // The server prices and broadcasts at the relay minimum when the
        // indexer has no estimate, as on regtest.
        match self.0.get_fee_rate(target).await {
            Err(EsploraError::NoFeeEstimate(_)) => Ok(fee::min_fee_rate(Chain::Bitcoin)),
            rate => Ok(rate?),
        }
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

#[cfg(test)]
mod tests {
    use bitcoin::{absolute::LockTime, transaction::Version};
    use serde_json::json;

    use super::*;
    use crate::esplora::tests::{answering, serving};

    fn tx() -> Transaction {
        Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![],
        }
    }

    #[tokio::test]
    async fn lists_an_output_reported_by_chain_and_mempool_once() {
        let address = Address::from_str("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4")
            .unwrap()
            .assume_checked();
        let txid = "f3aa4b63d7efaab2d562b896d2dffb835ceb8eb4bd7f3a2605fe99152217b3bf";
        let confirmed = json!({
            "confirmed": true,
            "block_height": 114,
            "block_hash": "544d75543fd180e4f1f96c5c4c3d86b3b0df1c8609f968daf1c20dd78b3f7012",
            "block_time": 1793893214,
        });
        let server = serving(
            &format!("/address/{address}/utxo"),
            json!([
                { "txid": txid, "vout": 0, "status": confirmed, "value": 101000 },
                { "txid": txid, "vout": 1, "status": confirmed, "value": 5000 },
                { "txid": txid, "vout": 0, "status": { "confirmed": false }, "value": 101000 },
            ]),
        )
        .await;

        let utxos = BitcoinEsplora::new(&server.uri())
            .unwrap()
            .address_utxos(&address)
            .await
            .unwrap();

        let vouts: Vec<u32> = utxos.iter().map(|utxo| utxo.vout).collect();
        assert_eq!(vouts, [0, 1]);
    }

    #[tokio::test]
    async fn asks_again_when_a_read_is_rate_limited() {
        let server = answering("/blocks/tip/height", &[429, 200]).await;

        let esplora = BitcoinEsplora::new(&server.uri()).unwrap();

        assert_eq!(esplora.get_height().await.unwrap(), 7);
    }

    #[tokio::test]
    async fn sends_a_broadcast_again_when_rate_limited() {
        let server = answering("/tx", &[429, 200]).await;

        let esplora = BitcoinEsplora::new(&server.uri()).unwrap();

        assert_eq!(esplora.broadcast(&tx()).await.unwrap(), tx().compute_txid());
    }

    #[tokio::test]
    async fn does_not_send_a_broadcast_again_on_a_server_error() {
        for status in [500, 503] {
            let server = answering("/tx", &[status]).await;

            let esplora = BitcoinEsplora::new(&server.uri()).unwrap();

            assert!(esplora.broadcast(&tx()).await.is_err());
        }
    }
}
