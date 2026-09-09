//! Liquid Esplora client and its chain-neutral [`ChainClient`] adapter.

use std::{collections::HashMap, str::FromStr};

use async_trait::async_trait;
use elements::{Address, AssetId, BlockHash, Transaction, TxOut, Txid, encode};
use serde::Deserialize;

use super::{AddressUtxo, ChainClient, ChainClientError, EsploraError, OutpointSpend, SpendingTx};

/// Elements' `minrelaytxfee` default, and the fallback when there is no estimate.
const MIN_FEE_RATE_SAT_PER_VB: f64 = 0.1;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct ApiTxStatus {
    confirmed: bool,
    block_height: Option<u32>,
}

impl ApiTxStatus {
    /// A response claiming confirmation without a height is treated as unconfirmed.
    fn confirmation_height(self) -> Option<u32> {
        self.block_height.filter(|_| self.confirmed)
    }
}

/// A spend as the explorer reports it, only when it names the spending txid
/// and input; anything less reads as unspent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiquidOutspend {
    pub txid: Txid,
    pub vin: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiquidUtxo {
    pub txid: Txid,
    pub vout: u32,
    pub value: Option<u64>,
    pub asset: Option<AssetId>,
}

#[derive(Debug, Clone)]
pub struct LiquidEsplora {
    client: reqwest::Client,
    base_url: String,
}

impl LiquidEsplora {
    pub fn new(url: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: url.trim_end_matches('/').to_owned(),
        }
    }

    pub async fn get_fee_rate(&self, target: usize) -> Result<f64, EsploraError> {
        let estimate = self
            .fee_estimates()
            .await
            .ok()
            .and_then(|estimates| convert_fee_rate(target, estimates))
            .unwrap_or(MIN_FEE_RATE_SAT_PER_VB);

        if !estimate.is_finite() || estimate < 0.0 {
            return Err(EsploraError::InvalidFeeRate(estimate));
        }
        Ok(estimate.max(MIN_FEE_RATE_SAT_PER_VB))
    }

    async fn fee_estimates(&self) -> Result<HashMap<u16, f64>, EsploraError> {
        Ok(self
            .client
            .get(format!("{}/fee-estimates", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    pub async fn get_tx_confirmations(&self, txid: &Txid) -> Result<Option<u32>, EsploraError> {
        let Some(status) = self.get_tx_status_optional(txid).await? else {
            return Ok(None);
        };
        let Some(block_height) = status.confirmation_height() else {
            return Ok(Some(0));
        };
        let tip = self.get_height().await?;
        Ok(Some(tip.saturating_sub(block_height) + 1))
    }

    pub async fn get_tx(&self, txid: &Txid) -> Result<Option<Transaction>, EsploraError> {
        let response = self
            .client
            .get(format!("{}/tx/{txid}/hex", self.base_url))
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let raw = response.error_for_status()?.text().await?;
        let bytes = hex::decode(raw.trim())
            .map_err(|e| EsploraError::InvalidResponse(format!("transaction hex: {e}")))?;
        let tx = encode::deserialize(&bytes)
            .map_err(|e| EsploraError::InvalidResponse(format!("transaction: {e}")))?;
        Ok(Some(tx))
    }

    pub async fn broadcast(&self, tx: &Transaction) -> Result<Txid, EsploraError> {
        let txid = tx.txid();
        self.client
            .post(format!("{}/tx", self.base_url))
            .body(hex::encode(encode::serialize(tx)))
            .send()
            .await?
            .error_for_status()?;
        Ok(txid)
    }

    pub async fn get_height(&self) -> Result<u32, EsploraError> {
        let raw = self
            .client
            .get(format!("{}/blocks/tip/height", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        raw.trim()
            .parse()
            .map_err(|e| EsploraError::InvalidResponse(format!("block height: {e}")))
    }

    /// Block hash at `height`, as reported by Esplora.
    pub async fn get_block_hash(&self, height: u32) -> Result<BlockHash, EsploraError> {
        let raw = self
            .client
            .get(format!("{}/block-height/{height}", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        parse_block_hash(raw.trim(), height)
    }

    pub async fn get_outspend(
        &self,
        txid: &Txid,
        vout: u32,
    ) -> Result<Option<LiquidOutspend>, EsploraError> {
        #[derive(Debug, Deserialize)]
        struct ApiOutspend {
            spent: bool,
            txid: Option<Txid>,
            vin: Option<u32>,
        }

        let response = self
            .client
            .get(format!("{}/tx/{txid}/outspend/{vout}", self.base_url))
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let outspend = response.error_for_status()?.json::<ApiOutspend>().await?;
        Ok(match (outspend.spent, outspend.txid, outspend.vin) {
            (true, Some(txid), Some(vin)) => Some(LiquidOutspend { txid, vin }),
            _ => None,
        })
    }

    pub async fn get_confirmation_height(&self, txid: &Txid) -> Result<Option<u32>, EsploraError> {
        self.get_tx_status_optional(txid)
            .await?
            .ok_or_else(|| EsploraError::InvalidResponse(format!("transaction {txid} not found")))
            .map(ApiTxStatus::confirmation_height)
    }

    async fn get_tx_status_optional(
        &self,
        txid: &Txid,
    ) -> Result<Option<ApiTxStatus>, EsploraError> {
        let response = self
            .client
            .get(format!("{}/tx/{txid}/status", self.base_url))
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Ok(Some(response.error_for_status()?.json().await?))
    }

    pub async fn address_utxos(&self, address: &Address) -> Result<Vec<LiquidUtxo>, EsploraError> {
        #[derive(Debug, Deserialize)]
        struct ApiUtxo {
            txid: Txid,
            vout: u32,
            #[serde(default)]
            value: Option<u64>,
            #[serde(default)]
            asset: Option<AssetId>,
        }

        let utxos = self
            .client
            .get(format!("{}/address/{address}/utxo", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .json::<Vec<ApiUtxo>>()
            .await?;
        Ok(utxos
            .into_iter()
            .map(|u| LiquidUtxo {
                txid: u.txid,
                vout: u.vout,
                value: u.value,
                asset: u.asset,
            })
            .collect())
    }

    /// Every transaction touching `address` with its confirmation height,
    /// newest first, paged to the end.
    pub async fn address_txs(
        &self,
        address: &Address,
    ) -> Result<Vec<(Txid, Option<u32>)>, EsploraError> {
        #[derive(Deserialize)]
        struct ApiTx {
            txid: Txid,
            status: ApiTxStatus,
        }

        let mut txs = Vec::new();
        let mut last_seen: Option<Txid> = None;
        loop {
            let path = match last_seen {
                Some(txid) => format!("{}/address/{address}/txs/chain/{txid}", self.base_url),
                None => format!("{}/address/{address}/txs", self.base_url),
            };
            let page: Vec<ApiTx> = self
                .client
                .get(path)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let confirmed = page.iter().filter(|tx| tx.status.confirmed).count();
            last_seen = page
                .iter()
                .rev()
                .find(|tx| tx.status.confirmed)
                .map(|tx| tx.txid);
            txs.extend(
                page.into_iter()
                    .map(|tx| (tx.txid, tx.status.confirmation_height())),
            );
            if confirmed < super::bitcoin::HISTORY_PAGE {
                return Ok(txs);
            }
        }
    }

    pub async fn get_prevout(&self, txid: &Txid, vout: u32) -> Result<TxOut, EsploraError> {
        let tx = self.get_tx(txid).await?.ok_or_else(|| {
            EsploraError::InvalidResponse(format!("transaction {txid} not found"))
        })?;
        tx.output.get(vout as usize).cloned().ok_or_else(|| {
            EsploraError::InvalidResponse(format!("output {vout} not found in {txid}"))
        })
    }
}

#[derive(Debug, Clone)]
pub struct LiquidChainClient(LiquidEsplora);

impl LiquidChainClient {
    pub fn new(url: &str) -> Self {
        Self(LiquidEsplora::new(url))
    }

    pub fn esplora(&self) -> &LiquidEsplora {
        &self.0
    }
}

#[async_trait]
impl ChainClient for LiquidChainClient {
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
            .map(|tx| hex::encode(encode::serialize(&tx))))
    }

    async fn broadcast(&self, tx_hex: &str) -> Result<String, ChainClientError> {
        let bytes = hex::decode(tx_hex)
            .map_err(|e| ChainClientError::Other(anyhow::anyhow!("invalid Liquid tx hex: {e}")))?;
        let tx: Transaction = encode::deserialize(&bytes).map_err(|e| {
            ChainClientError::Other(anyhow::anyhow!("invalid Liquid transaction: {e}"))
        })?;
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
                value: utxo.value,
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
        let tx_hex = hex::encode(encode::serialize(&tx));
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
        .map_err(|e| ChainClientError::Other(anyhow::anyhow!("invalid Liquid txid {txid}: {e}")))
}

fn parse_block_hash(value: &str, height: u32) -> Result<BlockHash, EsploraError> {
    BlockHash::from_str(value)
        .map_err(|e| EsploraError::InvalidResponse(format!("block hash at height {height}: {e}")))
}

fn parse_address(address: &str) -> Result<Address, ChainClientError> {
    Address::from_str(address).map_err(|e| {
        ChainClientError::Other(anyhow::anyhow!("invalid Liquid address {address}: {e}"))
    })
}

fn convert_fee_rate(target: usize, estimates: HashMap<u16, f64>) -> Option<f64> {
    estimates
        .into_iter()
        .filter(|(blocks, rate)| *blocks as usize <= target && rate.is_finite() && *rate >= 0.0)
        .max_by_key(|(blocks, _)| *blocks)
        .map(|(_, rate)| rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fee_estimate_uses_largest_target_not_exceeding_request() {
        let estimates = HashMap::from([(1, 5.0), (3, 2.0), (6, 1.0)]);
        assert_eq!(convert_fee_rate(1, estimates.clone()), Some(5.0));
        assert_eq!(convert_fee_rate(2, estimates.clone()), Some(5.0));
        assert_eq!(convert_fee_rate(3, estimates.clone()), Some(2.0));
        assert_eq!(convert_fee_rate(10, estimates), Some(1.0));
    }

    #[test]
    fn fee_estimate_ignores_invalid_rates() {
        let estimates = HashMap::from([(1, -1.0), (2, f64::NAN), (3, 2.0)]);
        assert_eq!(convert_fee_rate(2, estimates), None);
        assert_eq!(convert_fee_rate(3, HashMap::from([(3, 2.0)])), Some(2.0));
    }

    #[test]
    fn api_status_maps_only_confirmed_blocks() {
        assert_eq!(
            ApiTxStatus {
                confirmed: true,
                block_height: Some(42),
            }
            .confirmation_height(),
            Some(42)
        );
        assert_eq!(
            ApiTxStatus {
                confirmed: false,
                block_height: Some(42),
            }
            .confirmation_height(),
            None
        );
    }

    #[test]
    fn block_hash_parser_reports_the_height() {
        let error = parse_block_hash("not-a-block-hash", 0).unwrap_err();
        assert!(error.to_string().contains("block hash at height 0"));
    }
}
