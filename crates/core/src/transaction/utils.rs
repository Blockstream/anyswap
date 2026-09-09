//! Chain-dispatched, pure helpers over a signed-tx hex.
//!
//! Used on reload, where only the tx hex is persisted and the txid / miner fee
//! must be recomputed. Each helper takes the [`Chain`] and dispatches to the
//! bitcoin or liquid implementation.

use super::{TxError, bitcoin_tx_fee, decode_tx_hex, liquid};
use crate::types::Chain;

/// Txid of a signed tx hex.
pub fn compute_txid(chain: Chain, tx_hex: &str) -> Result<String, TxError> {
    match chain {
        Chain::Bitcoin => {
            let tx = decode_tx_hex(tx_hex)?;
            Ok(tx.compute_txid().to_string())
        }
        Chain::Liquid => {
            let tx = liquid::decode_tx_hex(tx_hex)?;
            Ok(tx.txid().to_string())
        }
    }
}

/// Miner fee of a signed tx hex.
///
/// `input_value` is only used for Bitcoin, where the fee is implicit
/// (inputs − outputs); Liquid records the fee in an explicit fee output, so the
/// input value is ignored.
pub fn tx_fee(chain: Chain, tx_hex: &str, input_value: u64) -> Result<u64, TxError> {
    match chain {
        Chain::Bitcoin => {
            let tx = decode_tx_hex(tx_hex)?;
            bitcoin_tx_fee(&tx, input_value)
        }
        Chain::Liquid => {
            let tx = liquid::decode_tx_hex(tx_hex)?;
            liquid::tx_fee(&tx)
        }
    }
}

/// Preimage revealed by the witness of input `vin` of a signed tx hex. `None`
/// unless the witness item hashes to `payment_hash`, so a witness that merely
/// looks like a claim cannot settle the invoice.
pub fn extract_preimage(
    chain: Chain,
    tx_hex: &str,
    vin: u32,
    payment_hash: &[u8; 32],
) -> Result<Option<[u8; 32]>, TxError> {
    match chain {
        Chain::Bitcoin => Ok(super::bitcoin::extract_preimage_at(
            &decode_tx_hex(tx_hex)?,
            vin,
            payment_hash,
        )),
        Chain::Liquid => Ok(liquid::extract_preimage_at(
            &liquid::decode_tx_hex(tx_hex)?,
            vin,
            payment_hash,
        )),
    }
}

/// Whether any output of a signed tx hex pays `address`. Compares script
/// pubkeys, so it works for Liquid confidential addresses too.
pub fn tx_pays_address(chain: Chain, tx_hex: &str, address: &str) -> Result<bool, TxError> {
    match chain {
        Chain::Bitcoin => {
            let tx = decode_tx_hex(tx_hex)?;
            let script = address
                .parse::<bitcoin::Address<bitcoin::address::NetworkUnchecked>>()
                .map_err(|e| TxError::InvalidDestination(e.to_string()))?
                .assume_checked()
                .script_pubkey();
            Ok(tx.output.iter().any(|o| o.script_pubkey == script))
        }
        Chain::Liquid => {
            let tx = liquid::decode_tx_hex(tx_hex)?;
            let script = address
                .parse::<elements::Address>()
                .map_err(|e| TxError::InvalidDestination(e.to_string()))?
                .script_pubkey();
            Ok(tx.output.iter().any(|o| o.script_pubkey == script))
        }
    }
}

#[cfg(test)]
mod tests {
    use elements::{LockTime, Transaction, TxOut, encode, issuance::AssetId};

    use super::*;

    #[test]
    fn liquid_reads_txid_and_explicit_fee() {
        let tx = Transaction {
            version: 2,
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![TxOut::new_fee(1_234, AssetId::LIQUID_BTC)],
        };
        let tx_hex = hex::encode(encode::serialize(&tx));

        assert_eq!(
            compute_txid(Chain::Liquid, &tx_hex).unwrap(),
            tx.txid().to_string()
        );
        // Unlike Bitcoin, Liquid's fee is explicit, so the input value is unused.
        assert_eq!(tx_fee(Chain::Liquid, &tx_hex, 0).unwrap(), 1_234);
    }
}
