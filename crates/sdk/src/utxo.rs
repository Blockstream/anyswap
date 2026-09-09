//! Outputs locked by a swap script.
//!
//! A [`SwapUtxo`] is what an indexer reports for one output: where it is, and
//! whatever the chain shows of its value. Bitcoin states the value outright;
//! Liquid keeps it blinded inside the output, so it is read on demand through
//! [`SwapIn::value`](crate::swap_in::SwapIn::value) or
//! [`SwapOut::value`](crate::swap_out::SwapOut::value), which hold the blinding
//! key.
//!
//! [`SwapIn::outputs`](crate::swap_in::SwapIn::outputs) and
//! [`SwapOut::verify_opening`](crate::swap_out::SwapOut::verify_opening) return
//! these; the refund and claim builders take them back. Build them yourself when
//! the outputs come from your own indexer rather than from a transaction the SDK
//! has seen.

use anyswap_core::{
    codec::{self, CodecError},
    fee,
    transaction::{self, liquid::liquid_prevout_secrets},
    types::{Chain, Outpoint},
};
use bitcoin::{Amount, secp256k1::SecretKey};
use elements::AssetId;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum UtxoError {
    #[error("expected a {expected} utxo, got a {found} one")]
    WrongChain { expected: Chain, found: Chain },

    #[error("invalid fee rate: {0} sat/vB")]
    InvalidFeeRate(f64),

    #[error(transparent)]
    Codec(#[from] CodecError),
}

/// An output of a swap script, as an indexer reports it.
#[derive(Clone, Debug)]
pub struct SwapUtxo {
    pub txid: String,
    pub vout: u32,
    kind: Kind,
}

/// What the chain shows of the output's value, which is where the two chains
/// differ: Bitcoin states it, Liquid blinds it inside the output.
#[derive(Clone, Debug)]
enum Kind {
    Bitcoin(Amount),
    Liquid(Box<elements::TxOut>),
}

impl SwapUtxo {
    /// A Bitcoin output, whose value the chain states.
    ///
    /// The value goes into the signature hash of any transaction spending this
    /// output, so one that disagrees with the funding transaction produces a
    /// transaction that will not relay.
    pub fn bitcoin(txid: impl Into<String>, vout: u32, value: u64) -> Self {
        Self {
            txid: txid.into(),
            vout,
            kind: Kind::Bitcoin(Amount::from_sat(value)),
        }
    }

    /// A Liquid output, whose value stays blinded until the swap unblinds it.
    pub fn liquid(txid: impl Into<String>, vout: u32, output: elements::TxOut) -> Self {
        Self {
            txid: txid.into(),
            vout,
            kind: Kind::Liquid(Box::new(output)),
        }
    }

    pub fn outpoint(&self) -> Outpoint {
        Outpoint::new(self.txid.clone(), self.vout)
    }

    pub fn chain(&self) -> Chain {
        match self.kind {
            Kind::Bitcoin(_) => Chain::Bitcoin,
            Kind::Liquid(_) => Chain::Liquid,
        }
    }

    /// The value in sats: stated for Bitcoin, unblinded with `liquid`'s key for
    /// Liquid. `None` when the output is not this swap's, meaning it is blinded
    /// to another key or holds another asset.
    pub(crate) fn value(&self, liquid: Option<(&SecretKey, AssetId)>) -> Option<u64> {
        match &self.kind {
            Kind::Bitcoin(value) => Some(value.to_sat()),
            Kind::Liquid(output) => {
                let (blinding_key, policy_asset) = liquid?;
                let secrets = liquid_prevout_secrets(output, blinding_key).ok()?;
                (secrets.asset == policy_asset).then_some(secrets.value)
            }
        }
    }
}

/// Turns `utxos` into the inputs the Bitcoin builders take.
pub(crate) fn bitcoin_inputs(
    utxos: &[SwapUtxo],
) -> Result<Vec<(bitcoin::OutPoint, Amount)>, UtxoError> {
    utxos
        .iter()
        .map(|utxo| match &utxo.kind {
            Kind::Bitcoin(value) => Ok((
                codec::parse_bitcoin_outpoint(&utxo.txid, utxo.vout)?,
                *value,
            )),
            Kind::Liquid(_) => Err(UtxoError::WrongChain {
                expected: Chain::Bitcoin,
                found: Chain::Liquid,
            }),
        })
        .collect()
}

/// Turns `utxos` into the inputs the Liquid builders take.
pub(crate) fn liquid_inputs(
    utxos: &[SwapUtxo],
) -> Result<Vec<(elements::OutPoint, elements::TxOut)>, UtxoError> {
    utxos
        .iter()
        .map(|utxo| match &utxo.kind {
            Kind::Liquid(output) => Ok((
                codec::parse_liquid_outpoint(&utxo.txid, utxo.vout)?,
                (**output).clone(),
            )),
            Kind::Bitcoin(_) => Err(UtxoError::WrongChain {
                expected: Chain::Liquid,
                found: Chain::Bitcoin,
            }),
        })
        .collect()
}

/// Splits `utxos` into the ones worth sweeping at `fee_rate` and the ones that
/// are not, so a refund does not spend more on fees than it recovers. An output
/// whose value cannot be read lands in the second set, since it cannot be
/// weighed.
///
/// Each output is weighed on its own against a whole single-input refund, and
/// against the CSV path, which is the larger of the two refund paths. Both
/// choices are deliberately conservative: a batch shares the fixed part of that
/// size between its inputs, so an output just under the line would in fact pay
/// for itself alongside others. Show the second set to the user rather than
/// dropping it silently.
pub(crate) fn partition_economical(
    utxos: &[SwapUtxo],
    chain: Chain,
    fee_rate: f64,
    liquid: Option<(SecretKey, AssetId)>,
) -> Result<(Vec<SwapUtxo>, Vec<SwapUtxo>), UtxoError> {
    // Liquid transactions are an order of magnitude larger than Bitcoin ones,
    // because confidential outputs carry commitments and rangeproofs.
    let vsize = match chain {
        Chain::Bitcoin => transaction::REFUND_VSIZE,
        Chain::Liquid => transaction::liquid::REFUND_VSIZE,
    };
    let fee =
        fee::fee_for_vsize(chain, vsize, fee_rate).ok_or(UtxoError::InvalidFeeRate(fee_rate))?;
    let liquid = liquid.as_ref().map(|(key, asset)| (key, *asset));
    Ok(utxos
        .iter()
        .cloned()
        .partition(|utxo| utxo.value(liquid).is_some_and(|value| value > fee)))
}
#[cfg(test)]
pub(crate) mod fixtures {
    use anyswap_core::script::LiquidSwapScript;
    use elements::{
        AddressParams, AssetId, BlockHash, LockTime, Transaction, TxOut, TxOutSecrets,
        confidential::{AssetBlindingFactor, ValueBlindingFactor},
        encode,
        secp256k1_zkp::Secp256k1 as LiquidSecp256k1,
    };

    /// A transaction paying `values` to `script`, each output blinded to the
    /// script's shared key, as the counterparty's opening would be.
    pub(crate) fn confidential_tx(script: &LiquidSwapScript, values: &[u64]) -> String {
        let policy_asset = AssetId::LIQUID_BTC;
        let mut rng = bitcoin::key::rand::thread_rng();
        let output = values
            .iter()
            .map(|value| {
                let secrets = TxOutSecrets::new(
                    policy_asset,
                    AssetBlindingFactor::zero(),
                    *value,
                    ValueBlindingFactor::zero(),
                );
                TxOut::new_last_confidential(
                    &mut rng,
                    &LiquidSecp256k1::new(),
                    *value,
                    policy_asset,
                    script.script_pubkey(),
                    script.blinding_pubkey(),
                    &[secrets],
                    &[],
                )
                .unwrap()
                .0
            })
            .collect();
        let tx = Transaction {
            version: 2,
            lock_time: LockTime::ZERO,
            input: vec![],
            output,
        };
        hex::encode(encode::serialize(&tx))
    }

    /// A confidential regtest address to sweep into.
    pub(crate) fn liquid_destination() -> String {
        let secp = LiquidSecp256k1::new();
        let key = elements::secp256k1_zkp::SecretKey::from_slice(&[9; 32]).unwrap();
        let blinding = elements::secp256k1_zkp::SecretKey::from_slice(&[10; 32]).unwrap();
        elements::Address::p2tr(
            &secp,
            key.x_only_public_key(&secp).0,
            None,
            Some(blinding.public_key(&secp)),
            &AddressParams::ELEMENTS,
        )
        .to_string()
    }

    pub(crate) fn liquid_asset() -> String {
        AssetId::LIQUID_BTC.to_string()
    }

    pub(crate) fn genesis_hash() -> String {
        BlockHash::from_raw_hash(bitcoin::hashes::Hash::from_byte_array([8; 32])).to_string()
    }
}
