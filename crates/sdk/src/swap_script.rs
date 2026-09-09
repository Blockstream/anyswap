//! The swap script of one on-chain side with everything spending it needs:
//! the chain, the network, and on Liquid the shared blinding key and chain
//! parameters. [`SwapIn`](crate::swap_in::SwapIn),
//! [`SwapOut`](crate::swap_out::SwapOut) and the driver all build their
//! transactions through here.

use std::fmt;
#[cfg(feature = "driver")]
use std::str::FromStr;

#[cfg(feature = "driver")]
use anyswap_core::types::Outpoint;
use anyswap_core::{
    script::{BitcoinSwapScript, LiquidSwapScript, derive_liquid_blinding_key},
    transaction::{
        self, ClaimByPreimageParams, KeypathSpendParams, LiquidChainCtx, RefundByCsvParams, TxError,
    },
    types::{Chain, SwapNetwork},
};
use bitcoin::secp256k1::{PublicKey, SecretKey};
use elements::AssetId;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    context::SwapContext,
    utxo::{self, SwapUtxo, UtxoError},
    wallet::SwapWallet,
};

#[derive(Debug, Error)]
pub enum ScriptError {
    #[error("this client is not set up for Liquid; add the Liquid parameters to the context")]
    LiquidNotConfigured,

    #[error("the swap trades asset {found}, but this client is set up for {expected}")]
    UnexpectedAsset { expected: String, found: String },

    #[error("missing field: {0}")]
    MissingField(&'static str),

    #[error("wallet: {0}")]
    Wallet(String),

    #[error("invalid swap script: {0}")]
    Script(String),
}

/// Which side of a swap an on-chain output is. `Source`: we refund and the
/// server claims. `Dest`: the reverse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Source,
    Dest,
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Source => "source",
            Self::Dest => "dest",
        })
    }
}

/// One on-chain side of a swap: what its script is built from. Our own key
/// and blinding key derive from `swap_id`, so only the server's are kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Terms {
    pub swap_id: Uuid,
    pub side: Side,
    pub chain: Chain,
    pub asset: String,
    pub csv: u32,
    pub payment_hash: [u8; 32],
    pub server_pubkey: PublicKey,
    pub server_blinding_pubkey: Option<PublicKey>,
}

/// A decoded transaction of either chain.
pub enum Tx {
    Bitcoin(bitcoin::Transaction),
    Liquid(elements::Transaction),
}

impl Tx {
    pub fn decode(chain: Chain, hex: &str) -> Result<Self, TxError> {
        Ok(match chain {
            Chain::Bitcoin => Self::Bitcoin(transaction::bitcoin::decode_tx_hex(hex)?),
            Chain::Liquid => Self::Liquid(transaction::liquid::decode_tx_hex(hex)?),
        })
    }

    pub fn txid(&self) -> String {
        match self {
            Self::Bitcoin(tx) => tx.compute_txid().to_string(),
            Self::Liquid(tx) => tx.txid().to_string(),
        }
    }

    #[cfg(feature = "driver")]
    pub fn inputs(&self) -> Vec<Outpoint> {
        let outpoint = |txid: String, vout| Outpoint::new(txid, vout);
        match self {
            Self::Bitcoin(tx) => tx
                .input
                .iter()
                .map(|i| outpoint(i.previous_output.txid.to_string(), i.previous_output.vout))
                .collect(),
            Self::Liquid(tx) => tx
                .input
                .iter()
                .map(|i| outpoint(i.previous_output.txid.to_string(), i.previous_output.vout))
                .collect(),
        }
    }

    #[cfg(feature = "driver")]
    pub fn preimage_at(&self, vin: u32, payment_hash: &[u8; 32]) -> Option<[u8; 32]> {
        match self {
            Self::Bitcoin(tx) => transaction::bitcoin::extract_preimage_at(tx, vin, payment_hash),
            Self::Liquid(tx) => transaction::liquid::extract_preimage_at(tx, vin, payment_hash),
        }
    }

    /// Whether any output pays `script_pubkey`.
    #[cfg(feature = "driver")]
    pub fn pays(&self, script_pubkey: &[u8]) -> bool {
        match self {
            Self::Bitcoin(tx) => tx
                .output
                .iter()
                .any(|o| o.script_pubkey.as_bytes() == script_pubkey),
            Self::Liquid(tx) => tx
                .output
                .iter()
                .any(|o| o.script_pubkey.as_bytes() == script_pubkey),
        }
    }
}

enum Inner {
    Bitcoin(Box<BitcoinSwapScript>),
    Liquid {
        script: Box<LiquidSwapScript>,
        blinding_key: SecretKey,
        ctx: LiquidChainCtx,
    },
}

pub struct SwapScript {
    chain: Chain,
    network: SwapNetwork,
    inner: Inner,
}

impl<W: SwapWallet> SwapContext<W> {
    /// The script of `terms`, with our key in the role the side gives it and,
    /// on Liquid, the blinding key shared with the server. Every Liquid path
    /// goes through here, so none can skip the asset check.
    pub fn script(&self, terms: &Terms) -> Result<SwapScript, ScriptError> {
        let wallet = self.wallet();
        let wallet_error = |e: W::Error| ScriptError::Wallet(e.to_string());
        let script_error = |e: anyhow::Error| ScriptError::Script(e.to_string());
        let ours = wallet.pubkey(&terms.swap_id).map_err(wallet_error)?;
        let (claim, refund) = match terms.side {
            Side::Source => (terms.server_pubkey, ours),
            Side::Dest => (ours, terms.server_pubkey),
        };
        let (claim, refund) = (claim.x_only_public_key().0, refund.x_only_public_key().0);
        let inner = match terms.chain {
            Chain::Bitcoin => Inner::Bitcoin(Box::new(
                BitcoinSwapScript::new(claim, refund, terms.payment_hash, terms.csv)
                    .map_err(script_error)?,
            )),
            Chain::Liquid => {
                let liquid = self.liquid().ok_or(ScriptError::LiquidNotConfigured)?;
                if terms.asset != liquid.policy_asset.to_string() {
                    return Err(ScriptError::UnexpectedAsset {
                        expected: liquid.policy_asset.to_string(),
                        found: terms.asset.clone(),
                    });
                }
                let own = match terms.side {
                    Side::Source => wallet.source_blinding_privkey(&terms.swap_id),
                    Side::Dest => wallet.dest_blinding_privkey(&terms.swap_id),
                }
                .map_err(wallet_error)?;
                let peer = terms
                    .server_blinding_pubkey
                    .ok_or(ScriptError::MissingField("server_blinding_pubkey"))?;
                let (blinding_pubkey, blinding_key) =
                    derive_liquid_blinding_key(&own, &peer).map_err(ScriptError::Script)?;
                let script = LiquidSwapScript::new(
                    claim,
                    refund,
                    terms.payment_hash,
                    terms.csv,
                    blinding_pubkey,
                )
                .map_err(script_error)?;
                Inner::Liquid {
                    script: Box::new(script),
                    blinding_key,
                    ctx: liquid,
                }
            }
        };
        Ok(SwapScript {
            chain: terms.chain,
            network: self.network(),
            inner,
        })
    }
}

impl SwapScript {
    pub fn address(&self) -> String {
        match &self.inner {
            Inner::Bitcoin(script) => script.address(self.network),
            Inner::Liquid { script, .. } => script.confidential_address(self.network).to_string(),
        }
    }

    /// The script pubkey of `address`, to recognise a payout to it.
    #[cfg(feature = "driver")]
    pub fn destination_script(&self, address: &str) -> Result<Vec<u8>, TxError> {
        let invalid = |e: &dyn std::fmt::Display| TxError::InvalidDestination(e.to_string());
        Ok(match self.chain {
            Chain::Bitcoin => bitcoin::Address::from_str(address)
                .map_err(|e| invalid(&e))?
                .assume_checked()
                .script_pubkey()
                .to_bytes(),
            Chain::Liquid => elements::Address::from_str(address)
                .map_err(|e| invalid(&e))?
                .script_pubkey()
                .to_bytes(),
        })
    }

    /// The outputs of `tx` that are this swap's: paying the script, and on
    /// Liquid unblinding to the policy asset with the shared key.
    pub fn outputs(&self, tx: &Tx) -> Vec<SwapUtxo> {
        let txid = tx.txid();
        match (&self.inner, tx) {
            (Inner::Bitcoin(script), Tx::Bitcoin(tx)) => {
                let spk = script.script_pubkey();
                tx.output
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| o.script_pubkey == spk)
                    .map(|(vout, o)| SwapUtxo::bitcoin(txid.clone(), vout as u32, o.value.to_sat()))
                    .collect()
            }
            (Inner::Liquid { script, .. }, Tx::Liquid(tx)) => {
                let spk = script.script_pubkey();
                tx.output
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| o.script_pubkey == spk)
                    .map(|(vout, o)| SwapUtxo::liquid(txid.clone(), vout as u32, o.clone()))
                    .filter(|utxo| self.value(utxo).is_some())
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// The output of `tx` at `vout`, checked to pay this script and, on
    /// Liquid, to hold its asset, with its value.
    pub fn output_at(&self, tx: &Tx, vout: u32) -> Result<(SwapUtxo, u64), TxError> {
        let txid = tx.txid();
        match (&self.inner, tx) {
            (Inner::Bitcoin(script), Tx::Bitcoin(tx)) => {
                let value = transaction::bitcoin::opening_value(tx, vout, script)?.to_sat();
                Ok((SwapUtxo::bitcoin(txid, vout, value), value))
            }
            (
                Inner::Liquid {
                    script,
                    blinding_key,
                    ctx,
                },
                Tx::Liquid(tx),
            ) => {
                let value = transaction::liquid::opening_value(
                    tx,
                    vout,
                    script,
                    blinding_key,
                    ctx.policy_asset,
                )?;
                let output = tx.output[vout as usize].clone();
                Ok((SwapUtxo::liquid(txid, vout, output), value))
            }
            _ => Err(TxError::InvalidPrevout(
                "transaction of another chain".into(),
            )),
        }
    }

    /// The value in sats, or `None` when the output is not this swap's.
    pub fn value(&self, utxo: &SwapUtxo) -> Option<u64> {
        utxo.value(self.unblinding())
    }

    /// Splits `utxos` into the ones worth spending at `fee_rate` and the rest.
    pub fn economical(
        &self,
        utxos: &[SwapUtxo],
        fee_rate: f64,
    ) -> Result<(Vec<SwapUtxo>, Vec<SwapUtxo>), UtxoError> {
        let liquid = self.unblinding().map(|(key, asset)| (*key, asset));
        utxo::partition_economical(utxos, self.chain, fee_rate, liquid)
    }

    /// A cooperative keypath spend, claim or refund, with both keys in hand.
    pub fn keypath(
        &self,
        utxos: &[SwapUtxo],
        claim_privkey: SecretKey,
        refund_privkey: SecretKey,
        destination: &str,
        fee_rate: f64,
    ) -> Result<String, TxError> {
        let params = match &self.inner {
            Inner::Bitcoin(script) => KeypathSpendParams::Bitcoin {
                inputs: utxo::bitcoin_inputs(utxos).map_err(wrong_chain)?,
                script: (**script).clone(),
                claim_privkey,
                refund_privkey,
            },
            Inner::Liquid {
                script,
                blinding_key,
                ctx,
            } => KeypathSpendParams::Liquid {
                inputs: utxo::liquid_inputs(utxos).map_err(wrong_chain)?,
                blinding_key: *blinding_key,
                script: (**script).clone(),
                genesis_hash: ctx.genesis_hash,
                policy_asset: ctx.policy_asset,
                claim_privkey,
                refund_privkey,
            },
        };
        transaction::build_refund_by_keypath(params, destination, fee_rate)
    }

    pub fn refund_by_csv(
        &self,
        utxos: &[SwapUtxo],
        refund_privkey: SecretKey,
        destination: &str,
        fee_rate: f64,
    ) -> Result<String, TxError> {
        let params = match &self.inner {
            Inner::Bitcoin(script) => RefundByCsvParams::Bitcoin {
                inputs: utxo::bitcoin_inputs(utxos).map_err(wrong_chain)?,
                script: (**script).clone(),
                refund_privkey,
            },
            Inner::Liquid {
                script,
                blinding_key,
                ctx,
            } => RefundByCsvParams::Liquid {
                inputs: utxo::liquid_inputs(utxos).map_err(wrong_chain)?,
                blinding_key: *blinding_key,
                script: (**script).clone(),
                genesis_hash: ctx.genesis_hash,
                policy_asset: ctx.policy_asset,
                refund_privkey,
            },
        };
        transaction::build_refund_by_csv(params, destination, fee_rate)
    }

    pub fn claim_by_preimage(
        &self,
        utxos: &[SwapUtxo],
        claim_privkey: SecretKey,
        preimage: [u8; 32],
        destination: &str,
        fee_rate: f64,
    ) -> Result<String, TxError> {
        let params = match &self.inner {
            Inner::Bitcoin(script) => ClaimByPreimageParams::Bitcoin {
                inputs: utxo::bitcoin_inputs(utxos).map_err(wrong_chain)?,
                script: (**script).clone(),
                claim_privkey,
                preimage,
            },
            Inner::Liquid {
                script,
                blinding_key,
                ctx,
            } => ClaimByPreimageParams::Liquid {
                inputs: utxo::liquid_inputs(utxos).map_err(wrong_chain)?,
                blinding_key: *blinding_key,
                script: (**script).clone(),
                genesis_hash: ctx.genesis_hash,
                policy_asset: ctx.policy_asset,
                claim_privkey,
                preimage,
            },
        };
        transaction::build_claim_by_preimage(params, destination, fee_rate)
    }

    #[cfg(test)]
    pub(crate) fn bitcoin(&self) -> Option<&BitcoinSwapScript> {
        match &self.inner {
            Inner::Bitcoin(script) => Some(script),
            Inner::Liquid { .. } => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn liquid(&self) -> Option<&LiquidSwapScript> {
        match &self.inner {
            Inner::Liquid { script, .. } => Some(script),
            Inner::Bitcoin(_) => None,
        }
    }

    fn unblinding(&self) -> Option<(&SecretKey, AssetId)> {
        match &self.inner {
            Inner::Bitcoin(_) => None,
            Inner::Liquid {
                blinding_key, ctx, ..
            } => Some((blinding_key, ctx.policy_asset)),
        }
    }
}

fn wrong_chain(e: UtxoError) -> TxError {
    TxError::InvalidPrevout(e.to_string())
}
