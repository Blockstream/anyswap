//! Transaction builders for the swap-script spending paths.
//!
//! Chain-neutral parameter enums dispatch to implementations in [`bitcoin`] and
//! [`liquid`].

use ::bitcoin::{Amount, OutPoint, secp256k1::SecretKey};
use thiserror::Error;

use crate::{
    musig::MusigError,
    script::{BitcoinSwapScript, LiquidSwapScript},
};

pub mod bitcoin;
mod context;
pub mod liquid;
pub mod utils;

pub(crate) use self::bitcoin::{
    bitcoin_tx_fee, decode_tx_hex, encode_tx_hex, parse_destination, sat_per_vb,
};
pub use self::{
    bitcoin::{CLAIM_VSIZE, KEYPATH_VSIZE, OPENING_VSIZE, REFUND_VSIZE, extract_preimage},
    context::{ChainCtx, LiquidChainCtx},
    liquid::{liquid_spend_input, validate_liquid_prevout},
};

#[derive(Debug, Error)]
pub enum TxError {
    #[error("no inputs")]
    NoInputs,

    #[error("duplicate input {0}")]
    DuplicateInput(String),

    #[error("input amount overflow")]
    InputOverflow,

    #[error("fee overflow")]
    FeeOverflow,

    #[error("fee {fee_sat} sat exceeds input value {input_sat} sat")]
    InsufficientValue { input_sat: u64, fee_sat: u64 },

    #[error("invalid sighash: {0}")]
    InvalidSighash(String),

    #[error("invalid destination address: {0}")]
    InvalidDestination(String),

    #[error("invalid fee rate: {0} sat/vB")]
    InvalidFeeRate(f64),

    #[error("output value {output_sat} sat is below the minimum {minimum_sat} sat")]
    DustOutput { output_sat: u64, minimum_sat: u64 },

    #[error("fee {fee_sat} sat is below required fee {required_sat} sat")]
    InsufficientFee { fee_sat: u64, required_sat: u64 },

    #[error("invalid prevout: {0}")]
    InvalidPrevout(String),

    #[error("unsupported Liquid asset {actual}; expected policy asset {policy_asset}")]
    UnsupportedLiquidAsset {
        actual: elements::issuance::AssetId,
        policy_asset: elements::issuance::AssetId,
    },

    #[error("invalid Liquid blinding factors: {0}")]
    InvalidBlindingFactors(String),

    #[error("invalid Liquid network: {0}")]
    InvalidLiquidNetwork(String),

    #[error("invalid Liquid transaction: {0}")]
    InvalidLiquidTransaction(String),

    #[error("invalid key: {0}")]
    InvalidKey(String),

    #[error("confidential output: {0}")]
    ConfidentialOutput(String),

    #[error("tx decode: {0}")]
    Decode(String),

    #[error(transparent)]
    Musig(#[from] MusigError),
}

/// Per-chain inputs for a cooperative keypath spend. All inputs must be UTXOs of
/// the same `script`; a single input is just the one-element case.
#[allow(clippy::large_enum_variant)]
pub enum KeypathSpendParams {
    Bitcoin {
        inputs: Vec<(OutPoint, Amount)>,
        script: BitcoinSwapScript,
        claim_privkey: SecretKey,
        refund_privkey: SecretKey,
    },
    Liquid {
        inputs: Vec<(elements::OutPoint, elements::TxOut)>,
        blinding_key: SecretKey,
        script: LiquidSwapScript,
        genesis_hash: elements::BlockHash,
        policy_asset: elements::AssetId,
        claim_privkey: SecretKey,
        refund_privkey: SecretKey,
    },
}

/// Inputs for a cooperative keypath claim.
pub type ClaimByKeypathParams = KeypathSpendParams;

/// Inputs for a cooperative keypath refund.
pub type RefundByKeypathParams = KeypathSpendParams;

/// Per-chain inputs for a script-path CSV refund (`refund_by_csv`). All inputs
/// must be UTXOs of the same `script` and must each have matured to its CSV depth.
#[allow(clippy::large_enum_variant)]
pub enum RefundByCsvParams {
    Bitcoin {
        inputs: Vec<(OutPoint, Amount)>,
        script: BitcoinSwapScript,
        refund_privkey: SecretKey,
    },
    Liquid {
        inputs: Vec<(elements::OutPoint, elements::TxOut)>,
        blinding_key: SecretKey,
        script: LiquidSwapScript,
        genesis_hash: elements::BlockHash,
        policy_asset: elements::AssetId,
        refund_privkey: SecretKey,
    },
}

/// Per-chain inputs for a script-path preimage claim (`claim_by_preimage`). All
/// inputs must be UTXOs of the same `script` and share the one preimage.
#[allow(clippy::large_enum_variant)]
pub enum ClaimByPreimageParams {
    Bitcoin {
        inputs: Vec<(OutPoint, Amount)>,
        script: BitcoinSwapScript,
        claim_privkey: SecretKey,
        preimage: [u8; 32],
    },
    Liquid {
        inputs: Vec<(elements::OutPoint, elements::TxOut)>,
        blinding_key: SecretKey,
        script: LiquidSwapScript,
        genesis_hash: elements::BlockHash,
        policy_asset: elements::AssetId,
        claim_privkey: SecretKey,
        preimage: [u8; 32],
    },
}

/// Builds and signs a cooperative keypath claim; returns the signed tx hex.
pub fn build_claim_by_keypath(
    params: ClaimByKeypathParams,
    destination: &str,
    fee_rate: f64,
) -> Result<String, TxError> {
    build_keypath_spend(params, destination, fee_rate)
}

/// Builds and signs a cooperative keypath refund; returns the signed tx hex.
pub fn build_refund_by_keypath(
    params: RefundByKeypathParams,
    destination: &str,
    fee_rate: f64,
) -> Result<String, TxError> {
    build_keypath_spend(params, destination, fee_rate)
}

fn build_keypath_spend(
    params: KeypathSpendParams,
    destination: &str,
    fee_rate: f64,
) -> Result<String, TxError> {
    match params {
        KeypathSpendParams::Bitcoin {
            inputs,
            script,
            claim_privkey,
            refund_privkey,
        } => {
            let tx = bitcoin::spend_by_keypath(
                &inputs,
                &script,
                &claim_privkey,
                &refund_privkey,
                &parse_destination(destination)?,
                sat_per_vb(fee_rate)?,
            )?;
            Ok(encode_tx_hex(&tx))
        }
        KeypathSpendParams::Liquid {
            inputs,
            blinding_key,
            script,
            genesis_hash,
            policy_asset,
            claim_privkey,
            refund_privkey,
        } => {
            let destination = liquid::parse_destination(destination)?;
            let tx = liquid::spend_by_keypath(
                &claim_privkey,
                &refund_privkey,
                &destination,
                fee_rate,
                &inputs,
                &blinding_key,
                script,
                genesis_hash,
                policy_asset,
            )?;
            Ok(liquid::encode_tx_hex(&tx))
        }
    }
}

/// Builds and signs a script-path CSV refund; returns the signed tx hex.
pub fn build_refund_by_csv(
    params: RefundByCsvParams,
    destination: &str,
    fee_rate: f64,
) -> Result<String, TxError> {
    match params {
        RefundByCsvParams::Bitcoin {
            inputs,
            script,
            refund_privkey,
        } => {
            let tx = bitcoin::refund_by_csv(
                &inputs,
                &script,
                &refund_privkey,
                &parse_destination(destination)?,
                sat_per_vb(fee_rate)?,
            )?;
            Ok(encode_tx_hex(&tx))
        }
        RefundByCsvParams::Liquid {
            inputs,
            blinding_key,
            script,
            genesis_hash,
            policy_asset,
            refund_privkey,
        } => {
            let destination = liquid::parse_destination(destination)?;
            let tx = liquid::refund_by_csv(
                &inputs,
                &blinding_key,
                &script,
                &refund_privkey,
                &destination,
                fee_rate,
                genesis_hash,
                policy_asset,
            )?;
            Ok(liquid::encode_tx_hex(&tx))
        }
    }
}

/// Builds and signs a script-path preimage claim; returns the signed tx hex.
pub fn build_claim_by_preimage(
    params: ClaimByPreimageParams,
    destination: &str,
    fee_rate: f64,
) -> Result<String, TxError> {
    match params {
        ClaimByPreimageParams::Bitcoin {
            inputs,
            script,
            claim_privkey,
            preimage,
        } => {
            let tx = bitcoin::claim_by_preimage(
                &inputs,
                &script,
                &claim_privkey,
                &preimage,
                &parse_destination(destination)?,
                sat_per_vb(fee_rate)?,
            )?;
            Ok(encode_tx_hex(&tx))
        }
        ClaimByPreimageParams::Liquid {
            inputs,
            blinding_key,
            script,
            genesis_hash,
            policy_asset,
            claim_privkey,
            preimage,
        } => {
            let destination = liquid::parse_destination(destination)?;
            let tx = liquid::claim_by_preimage(
                &inputs,
                &blinding_key,
                &script,
                &claim_privkey,
                &preimage,
                &destination,
                fee_rate,
                genesis_hash,
                policy_asset,
            )?;
            Ok(liquid::encode_tx_hex(&tx))
        }
    }
}
