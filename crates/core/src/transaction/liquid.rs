//! Liquid transaction builders for the [`LiquidSwapScript`] spending paths.
//!
//! Each builder spends one or more Liquid opening outputs locked by the same
//! [`LiquidSwapScript`] into one confidential destination output plus the
//! explicit policy-asset fee output required by Elements. A single input is just
//! the one-element case; passing several sweeps them all in one transaction (for
//! example, refunding a swap's deposit together with any stray deposits at the
//! same address). Each input's confidential value is recovered inside core by
//! unblinding the prevout with the shared `blinding_key`.

use std::{collections::HashSet, str::FromStr};

use bitcoin::secp256k1::SecretKey;
use elements::{
    Address, AssetIssuance, BlockHash, LockTime, OutPoint, SchnorrSighashType, Script, Sequence,
    Transaction, TxIn, TxInWitness, TxOut, TxOutSecrets,
    confidential::{Asset, AssetBlindingFactor, Value, ValueBlindingFactor},
    encode,
    hashes::Hash,
    issuance::AssetId,
    secp256k1_zkp::{Keypair, Message, Secp256k1},
    sighash::{Prevouts, SighashCache},
    taproot::TapLeafHash,
};

use super::TxError;
use crate::{fee::liquid::liquid_fee_for_tx, musig, script::LiquidSwapScript};

const LIQUID_TX_VERSION: u32 = 2;
const DUMMY_SCHNORR_SIG: [u8; 64] = [0; 64];
const DUMMY_PREIMAGE: [u8; 32] = [0; 32];

// Discounted vsizes for base-fee quotes, held against the builders by
// `quoted_vsizes_cover_the_built_transactions`.
/// Script-path claim size for base-fee quotes.
pub const CLAIM_VSIZE: u64 = 250;

/// Script-path CSV refund size for base-fee quotes.
pub const REFUND_VSIZE: u64 = 250;

/// Opening transaction size for base-fee quotes, covering the confidential
/// change output `elementsd` adds alongside the swap output.
pub const OPENING_VSIZE: u64 = 350;

/// Cooperative MuSig2 keypath spend, used for both claim and refund. All inputs
/// must be UTXOs of the same `script`, unblindable by the shared `blinding_key`.
#[allow(clippy::too_many_arguments)]
pub fn spend_by_keypath(
    claim_privkey: &SecretKey,
    refund_privkey: &SecretKey,
    destination: &Address,
    fee_rate: f64,
    inputs: &[(OutPoint, TxOut)],
    blinding_key: &SecretKey,
    script: LiquidSwapScript,
    genesis_hash: BlockHash,
    policy_asset: AssetId,
) -> Result<Transaction, TxError> {
    let secrets = resolve_inputs(inputs, blinding_key, &script, destination, policy_asset)?;
    let prevouts = prevouts(inputs);

    let tx = build_with_fee(fee_rate, |fee, estimate| {
        let mut tx = build_tx(
            inputs,
            build_destination_output(&secrets, destination, fee)?,
            fee,
            policy_asset,
            Sequence::ZERO,
        );
        if estimate {
            for txin in &mut tx.input {
                txin.witness.script_witness.push(DUMMY_SCHNORR_SIG.to_vec());
            }
        } else {
            let sighashes = keypath_sighashes(&tx, &prevouts, genesis_hash)?;
            let tweak = script.tap_tweak().to_byte_array();
            for (i, sighash) in sighashes.iter().enumerate() {
                let sig = musig::sign_keypath_with_tweak(
                    *claim_privkey,
                    *refund_privkey,
                    tweak,
                    sighash,
                )?
                .to_vec();
                tx.input[i].witness.script_witness.push(sig);
            }
        }
        Ok(tx)
    })?;

    validate_final_transaction(&tx, inputs, destination, Sequence::ZERO, 1, policy_asset)?;
    Ok(tx)
}

/// Script-path claim revealing the preimage. All inputs must be UTXOs of the
/// same `script` and share the one preimage.
#[allow(clippy::too_many_arguments)]
pub fn claim_by_preimage(
    inputs: &[(OutPoint, TxOut)],
    blinding_key: &SecretKey,
    script: &LiquidSwapScript,
    claim_privkey: &SecretKey,
    preimage: &[u8; 32],
    destination: &Address,
    fee_rate: f64,
    genesis_hash: BlockHash,
    policy_asset: AssetId,
) -> Result<Transaction, TxError> {
    let secrets = resolve_inputs(inputs, blinding_key, script, destination, policy_asset)?;
    let prevouts = prevouts(inputs);
    let control_block = script.claim_control_block().serialize();

    let tx = build_with_fee(fee_rate, |fee, estimate| {
        let mut tx = build_tx(
            inputs,
            build_destination_output(&secrets, destination, fee)?,
            fee,
            policy_asset,
            Sequence::ZERO,
        );
        let (sigs, witness_preimage): (Vec<Vec<u8>>, &[u8]) = if estimate {
            (
                vec![DUMMY_SCHNORR_SIG.to_vec(); inputs.len()],
                &DUMMY_PREIMAGE,
            )
        } else {
            let sighashes =
                script_path_sighashes(&tx, &prevouts, script.claim_leaf_hash(), genesis_hash)?;
            let sigs = sighashes
                .iter()
                .map(|sh| schnorr_sign(sh, claim_privkey))
                .collect::<Result<Vec<_>, _>>()?;
            (sigs, preimage)
        };
        for (i, sig) in sigs.into_iter().enumerate() {
            let witness = &mut tx.input[i].witness.script_witness;
            witness.push(sig);
            witness.push(witness_preimage.to_vec());
            witness.push(script.claim_script().as_bytes().to_vec());
            witness.push(control_block.clone());
        }
        Ok(tx)
    })?;

    validate_final_transaction(&tx, inputs, destination, Sequence::ZERO, 4, policy_asset)?;
    Ok(tx)
}

/// Script-path refund after the CSV timeout. All inputs must be UTXOs of the same
/// `script`; every input must have matured to its CSV depth for the transaction
/// to be final (the caller is responsible for enforcing maturity).
#[allow(clippy::too_many_arguments)]
pub fn refund_by_csv(
    inputs: &[(OutPoint, TxOut)],
    blinding_key: &SecretKey,
    script: &LiquidSwapScript,
    refund_privkey: &SecretKey,
    destination: &Address,
    fee_rate: f64,
    genesis_hash: BlockHash,
    policy_asset: AssetId,
) -> Result<Transaction, TxError> {
    let secrets = resolve_inputs(inputs, blinding_key, script, destination, policy_asset)?;
    let prevouts = prevouts(inputs);
    let control_block = script.refund_control_block().serialize();
    let sequence = Sequence::from_consensus(script.csv_blocks());

    let tx = build_with_fee(fee_rate, |fee, estimate| {
        let mut tx = build_tx(
            inputs,
            build_destination_output(&secrets, destination, fee)?,
            fee,
            policy_asset,
            sequence,
        );
        let sigs: Vec<Vec<u8>> = if estimate {
            vec![DUMMY_SCHNORR_SIG.to_vec(); inputs.len()]
        } else {
            let sighashes =
                script_path_sighashes(&tx, &prevouts, script.refund_leaf_hash(), genesis_hash)?;
            sighashes
                .iter()
                .map(|sh| schnorr_sign(sh, refund_privkey))
                .collect::<Result<Vec<_>, _>>()?
        };
        for (i, sig) in sigs.into_iter().enumerate() {
            let witness = &mut tx.input[i].witness.script_witness;
            witness.push(sig);
            witness.push(script.refund_script().as_bytes().to_vec());
            witness.push(control_block.clone());
        }
        Ok(tx)
    })?;

    validate_final_transaction(&tx, inputs, destination, sequence, 3, policy_asset)?;
    Ok(tx)
}

/// Unblinds and validates every batch input against the shared blinding key and
/// the swap script, rejecting an empty set or a duplicate outpoint. Returns the
/// per-input secrets in input order.
fn resolve_inputs(
    inputs: &[(OutPoint, TxOut)],
    blinding_key: &SecretKey,
    script: &LiquidSwapScript,
    destination: &Address,
    policy_asset: AssetId,
) -> Result<Vec<TxOutSecrets>, TxError> {
    if inputs.is_empty() {
        return Err(TxError::NoInputs);
    }
    if inputs.len() > 1 {
        let mut seen = HashSet::with_capacity(inputs.len());
        for (outpoint, _) in inputs {
            if !seen.insert(*outpoint) {
                return Err(TxError::DuplicateInput(outpoint.to_string()));
            }
        }
    }
    inputs
        .iter()
        .map(|(_, prevout)| {
            let secrets = liquid_prevout_secrets(prevout, blinding_key)?;
            validate_prevout(prevout, &secrets, script, destination, policy_asset)?;
            Ok(secrets)
        })
        .collect()
}

fn prevouts(inputs: &[(OutPoint, TxOut)]) -> Vec<TxOut> {
    inputs.iter().map(|(_, prevout)| prevout.clone()).collect()
}

fn validate_prevout(
    prevout: &TxOut,
    secrets: &TxOutSecrets,
    script: &LiquidSwapScript,
    destination: &Address,
    policy_asset: AssetId,
) -> Result<(), TxError> {
    if secrets.asset != policy_asset {
        return Err(TxError::UnsupportedLiquidAsset {
            actual: secrets.asset,
            policy_asset,
        });
    }

    if destination.blinding_pubkey.is_none() {
        return Err(TxError::ConfidentialOutput(
            "destination must be confidential".into(),
        ));
    }
    if prevout.script_pubkey != script.script_pubkey() {
        return Err(TxError::InvalidPrevout("script_pubkey mismatch".into()));
    }

    validate_explicit_blinding_factors(prevout, secrets)?;
    if !prevout_asset_matches(prevout.asset, secrets) {
        return Err(TxError::InvalidPrevout("asset mismatch".into()));
    }
    if !prevout_value_matches(prevout.value, secrets) {
        return Err(TxError::InvalidPrevout("value mismatch".into()));
    }
    Ok(())
}

fn validate_explicit_blinding_factors(
    prevout: &TxOut,
    secrets: &TxOutSecrets,
) -> Result<(), TxError> {
    if prevout.asset.is_explicit() && secrets.asset_bf != AssetBlindingFactor::zero() {
        return Err(TxError::InvalidBlindingFactors(
            "asset_bf must be zero when prevout.asset is explicit".into(),
        ));
    }
    if prevout.value.is_explicit() && secrets.value_bf != ValueBlindingFactor::zero() {
        return Err(TxError::InvalidBlindingFactors(
            "value_bf must be zero when prevout.value is explicit".into(),
        ));
    }
    Ok(())
}

fn prevout_asset_matches(asset: Asset, secrets: &TxOutSecrets) -> bool {
    match asset {
        Asset::Explicit(asset) => asset == secrets.asset,
        Asset::Confidential(_) => {
            let secp = elements::secp256k1_zkp::Secp256k1::new();
            asset == Asset::new_confidential(&secp, secrets.asset, secrets.asset_bf)
        }
        Asset::Null => false,
    }
}

fn prevout_value_matches(value: Value, secrets: &TxOutSecrets) -> bool {
    match value {
        Value::Explicit(value) => value == secrets.value,
        Value::Confidential(_) => {
            let secp = elements::secp256k1_zkp::Secp256k1::new();
            value
                == Value::new_confidential_from_assetid(
                    &secp,
                    secrets.value,
                    secrets.asset,
                    secrets.value_bf,
                    secrets.asset_bf,
                )
        }
        Value::Null => false,
    }
}

/// Builds the single confidential destination output paying `Σ secrets − fee`,
/// balanced against every input's blinding factors. `secrets` is non-empty and
/// all of the policy asset (enforced by [`resolve_inputs`]).
fn build_destination_output(
    secrets: &[TxOutSecrets],
    destination: &Address,
    fee: u64,
) -> Result<TxOut, TxError> {
    let total = secrets
        .iter()
        .try_fold(0_u64, |acc, s| acc.checked_add(s.value))
        .ok_or(TxError::InputOverflow)?;
    let value = total.checked_sub(fee).ok_or(TxError::InsufficientValue {
        input_sat: total,
        fee_sat: fee,
    })?;
    if value == 0 {
        return Err(TxError::DustOutput {
            output_sat: value,
            minimum_sat: 1,
        });
    }

    let asset = secrets[0].asset;
    let blinding_pubkey = destination
        .blinding_pubkey
        .ok_or_else(|| TxError::ConfidentialOutput("destination must be confidential".into()))?;
    let secp = elements::secp256k1_zkp::Secp256k1::new();
    let mut rng = bitcoin::key::rand::thread_rng();
    let (txout, _, _, _) = TxOut::new_last_confidential(
        &mut rng,
        &secp,
        value,
        asset,
        destination.script_pubkey(),
        blinding_pubkey,
        secrets,
        &[],
    )
    .map_err(|e| TxError::ConfidentialOutput(e.to_string()))?;
    Ok(txout)
}

fn build_tx(
    inputs: &[(OutPoint, TxOut)],
    destination_output: TxOut,
    fee: u64,
    policy_asset: AssetId,
    sequence: Sequence,
) -> Transaction {
    Transaction {
        version: LIQUID_TX_VERSION,
        lock_time: LockTime::ZERO,
        input: inputs
            .iter()
            .map(|(outpoint, _)| TxIn {
                previous_output: *outpoint,
                is_pegin: false,
                script_sig: Script::new(),
                sequence,
                asset_issuance: AssetIssuance::default(),
                witness: TxInWitness::empty(),
            })
            .collect(),
        output: vec![destination_output, TxOut::new_fee(fee, policy_asset)],
    }
}

fn build_with_fee(
    fee_rate: f64,
    build: impl Fn(u64, bool) -> Result<Transaction, TxError>,
) -> Result<Transaction, TxError> {
    let estimated = build(0, true)?;
    let fee = liquid_fee_for_tx(&estimated, fee_rate)?;
    let tx = build(fee, false)?;
    let required_fee = liquid_fee_for_tx(&tx, fee_rate)?;
    if fee < required_fee {
        return Err(TxError::InsufficientFee {
            fee_sat: fee,
            required_sat: required_fee,
        });
    }
    Ok(tx)
}

/// Taproot key-spend sighashes, one per input, each committing to every prevout.
fn keypath_sighashes(
    tx: &Transaction,
    prevouts: &[TxOut],
    genesis_hash: BlockHash,
) -> Result<Vec<[u8; 32]>, TxError> {
    let mut cache = SighashCache::new(tx);
    (0..prevouts.len())
        .map(|i| {
            cache
                .taproot_key_spend_signature_hash(
                    i,
                    &Prevouts::All(prevouts),
                    SchnorrSighashType::Default,
                    genesis_hash,
                )
                .map(|sh| sh.to_byte_array())
                .map_err(|e| TxError::InvalidSighash(e.to_string()))
        })
        .collect()
}

/// Taproot script-spend sighashes for `leaf_hash`, one per input, each
/// committing to every prevout.
fn script_path_sighashes(
    tx: &Transaction,
    prevouts: &[TxOut],
    leaf_hash: TapLeafHash,
    genesis_hash: BlockHash,
) -> Result<Vec<[u8; 32]>, TxError> {
    let mut cache = SighashCache::new(tx);
    (0..prevouts.len())
        .map(|i| {
            cache
                .taproot_script_spend_signature_hash(
                    i,
                    &Prevouts::All(prevouts),
                    leaf_hash,
                    SchnorrSighashType::Default,
                    genesis_hash,
                )
                .map(|sh| sh.to_byte_array())
                .map_err(|e| TxError::InvalidSighash(e.to_string()))
        })
        .collect()
}

fn schnorr_sign(sighash: &[u8; 32], privkey: &SecretKey) -> Result<Vec<u8>, TxError> {
    let secp = Secp256k1::new();
    let secret_key = elements::secp256k1_zkp::SecretKey::from_slice(&privkey.secret_bytes())
        .map_err(|e| TxError::InvalidKey(e.to_string()))?;
    let keypair = Keypair::from_secret_key(&secp, &secret_key);
    let message = Message::from_digest(*sighash);
    Ok(secp.sign_schnorr(&message, &keypair).as_ref().to_vec())
}

fn validate_final_transaction(
    tx: &Transaction,
    inputs: &[(OutPoint, TxOut)],
    destination: &Address,
    expected_sequence: Sequence,
    expected_witness_items: usize,
    policy_asset: AssetId,
) -> Result<(), TxError> {
    let invalid = |reason: &str| TxError::InvalidLiquidTransaction(reason.into());

    if tx.version != LIQUID_TX_VERSION || tx.lock_time != LockTime::ZERO {
        return Err(invalid("unexpected version or lock_time"));
    }

    if tx.input.len() != inputs.len() || tx.output.len() != 2 {
        return Err(invalid("unexpected input or output count"));
    }

    for (input, (outpoint, _)) in tx.input.iter().zip(inputs) {
        if input.previous_output != *outpoint {
            return Err(invalid("input outpoint mismatch"));
        }
        if input.is_pegin
            || !input.script_sig.is_empty()
            || input.sequence != expected_sequence
            || !input.asset_issuance.is_null()
            || input.witness.amount_rangeproof.is_some()
            || input.witness.inflation_keys_rangeproof.is_some()
            || !input.witness.pegin_witness.is_empty()
        {
            return Err(invalid("unexpected input fields"));
        }
        if input.witness.script_witness.len() != expected_witness_items
            || input
                .witness
                .script_witness
                .first()
                .is_none_or(|signature| signature.len() != 64)
        {
            return Err(invalid("unexpected taproot witness"));
        }
    }

    let destination_output = &tx.output[0];
    if destination_output.script_pubkey != destination.script_pubkey()
        || !destination_output.asset.is_confidential()
        || !destination_output.value.is_confidential()
        || !destination_output.nonce.is_confidential()
        || destination_output.witness.surjection_proof.is_none()
        || destination_output.witness.rangeproof.is_none()
    {
        return Err(invalid("invalid confidential destination output"));
    }

    let fee_output = &tx.output[1];
    if !fee_output.is_fee()
        || fee_output.asset != Asset::Explicit(policy_asset)
        || !fee_output.value.is_explicit()
        || !fee_output.nonce.is_null()
        || !fee_output.witness.is_empty()
    {
        return Err(invalid("invalid policy-asset fee output"));
    }

    let secp = elements::secp256k1_zkp::Secp256k1::new();
    tx.verify_tx_amt_proofs(&secp, &prevouts(inputs))
        .map_err(|e| TxError::InvalidLiquidTransaction(e.to_string()))?;
    Ok(())
}

pub(crate) fn parse_destination(destination: &str) -> Result<Address, TxError> {
    Address::from_str(destination).map_err(|e| TxError::InvalidDestination(e.to_string()))
}

pub(crate) fn encode_tx_hex(tx: &Transaction) -> String {
    hex::encode(encode::serialize(tx))
}

pub fn decode_tx_hex(tx_hex: &str) -> Result<Transaction, TxError> {
    let bytes = hex::decode(tx_hex).map_err(|e| TxError::Decode(e.to_string()))?;
    encode::deserialize(&bytes).map_err(|e| TxError::Decode(e.to_string()))
}

/// Reads the explicit fee output of a Liquid transaction.
pub fn tx_fee(tx: &Transaction) -> Result<u64, TxError> {
    let mut found = false;
    let fee =
        tx.output
            .iter()
            .filter(|output| output.is_fee())
            .try_fold(0_u64, |total, output| {
                found = true;
                let value = output.value.explicit().ok_or_else(|| {
                    TxError::InvalidLiquidTransaction("fee output must be explicit".into())
                })?;
                total.checked_add(value).ok_or(TxError::FeeOverflow)
            })?;
    if !found {
        return Err(TxError::InvalidLiquidTransaction(
            "transaction has no fee output".into(),
        ));
    }
    Ok(fee)
}

/// Finds the first output paying `script_pubkey` and returns its vout.
pub fn output_vout(tx: &Transaction, script_pubkey: &Script) -> Result<u32, TxError> {
    tx.output
        .iter()
        .position(|output| output.script_pubkey == *script_pubkey)
        .map(|vout| vout as u32)
        .ok_or_else(|| TxError::InvalidPrevout("output matching script_pubkey not found".into()))
}

/// Extracts a preimage from `[sig, preimage, claim_script, control_block]`.
pub fn extract_preimage(tx: &Transaction, payment_hash: &[u8; 32]) -> Option<[u8; 32]> {
    extract_preimage_at(tx, 0, payment_hash)
}

/// Extracts a preimage from the witness of the specified transaction input.
/// Returns `None` unless the witness item hashes to `payment_hash`, so a
/// witness that merely looks like a claim cannot settle the invoice.
pub fn extract_preimage_at(
    tx: &Transaction,
    vin: u32,
    payment_hash: &[u8; 32],
) -> Option<[u8; 32]> {
    let preimage: [u8; 32] = tx
        .input
        .get(vin as usize)?
        .witness
        .script_witness
        .get(1)?
        .as_slice()
        .try_into()
        .ok()?;
    (crate::utils::sha256_payment_hash(&preimage) == *payment_hash).then_some(preimage)
}

/// Extracts the prevout at `outpoint` from its opening `tx`, checking that the
/// transaction matches the outpoint's txid and the vout is in range. Performs no
/// swap-script or asset validation and does not unblind.
pub fn liquid_prevout(tx: &Transaction, outpoint: OutPoint) -> Result<TxOut, TxError> {
    let txid = tx.txid();
    if txid != outpoint.txid {
        return Err(TxError::InvalidPrevout(format!(
            "expected txid {}, got {txid}",
            outpoint.txid
        )));
    }
    tx.output
        .get(outpoint.vout as usize)
        .cloned()
        .ok_or_else(|| TxError::InvalidPrevout(format!("vout {} out of range", outpoint.vout)))
}

/// Collects the Liquid spend input at `outpoint`: extracts the prevout with
/// [`liquid_prevout`] and unblinds it with `blinding_key`.
///
/// It performs no swap-script or asset validation. Callers that spend the input
/// through the builders can rely on their re-validation; a caller relying on an
/// opening it does not control (e.g. a client verifying the server's funding
/// transaction) must additionally run [`validate_liquid_prevout`].
pub fn liquid_spend_input(
    tx: &Transaction,
    outpoint: OutPoint,
    blinding_key: &SecretKey,
) -> Result<(TxOut, TxOutSecrets), TxError> {
    let prevout = liquid_prevout(tx, outpoint)?;
    let prevout_secrets = liquid_prevout_secrets(&prevout, blinding_key)?;
    Ok((prevout, prevout_secrets))
}

/// Validates a collected prevout against the swap `script` and `policy_asset`.
/// Run this before trusting an opening whose funder is not trusted - the chain
/// data being faithful does not imply the counterparty funded the agreed script
/// and asset.
pub fn validate_liquid_prevout(
    prevout: &TxOut,
    secrets: &TxOutSecrets,
    script: &LiquidSwapScript,
    policy_asset: AssetId,
) -> Result<(), TxError> {
    if prevout.script_pubkey != script.script_pubkey() {
        return Err(TxError::InvalidPrevout("script_pubkey mismatch".into()));
    }
    if secrets.asset != policy_asset {
        return Err(TxError::UnsupportedLiquidAsset {
            actual: secrets.asset,
            policy_asset,
        });
    }
    Ok(())
}

/// Resolves the funded value of the opening output at `vout`, unblinding it with
/// `blinding_key` and confirming it pays `script` in `policy_asset`. The caller
/// compares the returned value against the agreed amount.
pub fn opening_value(
    tx: &Transaction,
    vout: u32,
    script: &LiquidSwapScript,
    blinding_key: &SecretKey,
    policy_asset: AssetId,
) -> Result<u64, TxError> {
    let prevout = tx
        .output
        .get(vout as usize)
        .cloned()
        .ok_or_else(|| TxError::InvalidPrevout(format!("vout {vout} out of range")))?;
    let secrets = liquid_prevout_secrets(&prevout, blinding_key)?;
    validate_liquid_prevout(&prevout, &secrets, script, policy_asset)?;
    Ok(secrets.value)
}

/// Unblinds `prevout` with `blinding_key`, or returns its explicit secrets
/// directly when the output is unblinded.
pub fn liquid_prevout_secrets(
    prevout: &TxOut,
    blinding_key: &SecretKey,
) -> Result<TxOutSecrets, TxError> {
    match (prevout.asset.explicit(), prevout.value.explicit()) {
        (Some(asset), Some(value)) => Ok(TxOutSecrets::new(
            asset,
            AssetBlindingFactor::zero(),
            value,
            ValueBlindingFactor::zero(),
        )),
        _ => {
            let secp = Secp256k1::new();
            let blinding_key = SecretKey::from_slice(&blinding_key.secret_bytes())
                .map_err(|e| TxError::InvalidKey(e.to_string()))?;
            prevout
                .unblind(&secp, blinding_key)
                .map_err(|e| TxError::InvalidBlindingFactors(e.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        hashes::{Hash as _, sha256},
        secp256k1::XOnlyPublicKey as BitcoinXOnlyPublicKey,
    };
    use elements::{
        AddressParams, Txid,
        confidential::{AssetBlindingFactor, Nonce, ValueBlindingFactor},
        schnorr::XOnlyPublicKey,
        secp256k1_zkp::{PublicKey, Secp256k1, SecretKey as LiquidSecretKey, schnorr::Signature},
    };

    use super::*;

    struct Fixture {
        outpoint: OutPoint,
        prevout: TxOut,
        secrets: TxOutSecrets,
        script: LiquidSwapScript,
        opening_blinding_key: LiquidSecretKey,
        destination: Address,
        destination_blinding_key: LiquidSecretKey,
        genesis_hash: BlockHash,
        claim_key: SecretKey,
        refund_key: SecretKey,
        preimage: [u8; 32],
        fee_rate: f64,
    }

    fn bitcoin_key(seed: u8) -> (SecretKey, BitcoinXOnlyPublicKey) {
        let secret_key = SecretKey::from_slice(&[seed; 32]).unwrap();
        let public_key = secret_key.x_only_public_key(crate::secp()).0;
        (secret_key, public_key)
    }

    fn liquid_public_key(seed: u8) -> (LiquidSecretKey, PublicKey) {
        let secp = Secp256k1::new();
        let secret_key = LiquidSecretKey::from_slice(&[seed; 32]).unwrap();
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);
        (secret_key, public_key)
    }

    fn fixture() -> Fixture {
        let (claim_key, claim_pubkey) = bitcoin_key(1);
        let (refund_key, refund_pubkey) = bitcoin_key(2);
        let (opening_blinding_key, opening_blinding_pubkey) = liquid_public_key(3);
        let (destination_blinding_key, destination_blinding_pubkey) = liquid_public_key(4);
        let destination_spend_key = LiquidSecretKey::from_slice(&[5; 32]).unwrap();
        let secp = Secp256k1::new();
        let destination_xonly = destination_spend_key.x_only_public_key(&secp).0;
        let destination = Address::p2tr(
            &secp,
            destination_xonly,
            None,
            Some(destination_blinding_pubkey),
            &AddressParams::LIQUID_TESTNET,
        );
        let preimage = [42; 32];
        let payment_hash = sha256::Hash::hash(&preimage).to_byte_array();
        let script = LiquidSwapScript::new(
            claim_pubkey,
            refund_pubkey,
            payment_hash,
            144,
            opening_blinding_pubkey,
        )
        .unwrap();
        let policy_asset = AssetId::LIQUID_BTC;
        let secrets = TxOutSecrets::new(
            policy_asset,
            AssetBlindingFactor::zero(),
            100_000,
            ValueBlindingFactor::zero(),
        );
        let prevout = TxOut {
            asset: Asset::Explicit(policy_asset),
            value: Value::Explicit(secrets.value),
            nonce: Nonce::Null,
            script_pubkey: script.script_pubkey(),
            witness: Default::default(),
        };

        Fixture {
            outpoint: OutPoint::new(Txid::from_byte_array([7; 32]), 0),
            prevout,
            secrets,
            script,
            opening_blinding_key,
            destination,
            destination_blinding_key,
            genesis_hash: BlockHash::from_byte_array([8; 32]),
            claim_key,
            refund_key,
            preimage,
            fee_rate: 1.0,
        }
    }

    /// The shared blinding key as the `bitcoin` secret key the builders take.
    fn blinding_key(fixture: &Fixture) -> SecretKey {
        SecretKey::from_slice(&fixture.opening_blinding_key.secret_bytes()).unwrap()
    }

    /// The fixture's single explicit opening as a one-element input slice.
    fn single_input(fixture: &Fixture) -> Vec<(OutPoint, TxOut)> {
        vec![(fixture.outpoint, fixture.prevout.clone())]
    }

    /// A confidential swap-address output of `value`, unblindable by the shared
    /// opening blinding key.
    fn confidential_swap_output(fixture: &Fixture, value: u64) -> TxOut {
        let secp = Secp256k1::new();
        let mut rng = bitcoin::key::rand::thread_rng();
        let secrets = TxOutSecrets::new(
            fixture.secrets.asset,
            AssetBlindingFactor::zero(),
            value,
            ValueBlindingFactor::zero(),
        );
        let (output, _, _, _) = TxOut::new_last_confidential(
            &mut rng,
            &secp,
            value,
            secrets.asset,
            fixture.script.script_pubkey(),
            fixture.script.blinding_pubkey(),
            &[secrets],
            &[],
        )
        .unwrap();
        output
    }

    fn assert_common_shape(tx: &Transaction, fixture: &Fixture, witness_items: usize) {
        assert_eq!(tx.input.len(), 1);
        assert_eq!(tx.output.len(), 2);
        assert_eq!(tx.input[0].previous_output, fixture.outpoint);
        assert_eq!(tx.input[0].witness.script_witness.len(), witness_items);
        assert_eq!(tx.output[1].asset, Asset::Explicit(AssetId::LIQUID_BTC));
        let fee = tx_fee(tx).unwrap();
        assert_eq!(
            fee as f64,
            (tx.discount_vsize() as f64 * fixture.fee_rate).ceil()
        );

        let secp = Secp256k1::new();
        let destination_secrets = tx.output[0]
            .unblind(&secp, fixture.destination_blinding_key)
            .unwrap();
        assert_eq!(destination_secrets.asset, fixture.secrets.asset);
        assert_eq!(destination_secrets.value, fixture.secrets.value - fee);
        tx.verify_tx_amt_proofs(&secp, std::slice::from_ref(&fixture.prevout))
            .unwrap();
    }

    fn keypath_sighash(tx: &Transaction, fixture: &Fixture) -> [u8; 32] {
        SighashCache::new(tx)
            .taproot_key_spend_signature_hash(
                0,
                &Prevouts::All(&[&fixture.prevout]),
                SchnorrSighashType::Default,
                fixture.genesis_hash,
            )
            .unwrap()
            .to_byte_array()
    }

    fn verify_signature(signature: &[u8], message: [u8; 32], public_key: &XOnlyPublicKey) {
        let secp = Secp256k1::new();
        secp.verify_schnorr(
            &Signature::from_slice(signature).unwrap(),
            &Message::from_digest(message),
            public_key,
        )
        .unwrap();
    }

    #[test]
    fn keypath_spend_is_signed_and_balanced() {
        let fixture = fixture();
        let tx = spend_by_keypath(
            &fixture.claim_key,
            &fixture.refund_key,
            &fixture.destination,
            fixture.fee_rate,
            &single_input(&fixture),
            &blinding_key(&fixture),
            fixture.script.clone(),
            fixture.genesis_hash,
            AssetId::LIQUID_BTC,
        )
        .unwrap();

        assert_common_shape(&tx, &fixture, 1);
        verify_signature(
            &tx.input[0].witness.script_witness[0],
            keypath_sighash(&tx, &fixture),
            fixture.script.output_key().as_inner(),
        );
    }

    #[test]
    fn preimage_claim_has_expected_witness_and_signature() {
        let fixture = fixture();
        let tx = claim_by_preimage(
            &single_input(&fixture),
            &blinding_key(&fixture),
            &fixture.script,
            &fixture.claim_key,
            &fixture.preimage,
            &fixture.destination,
            fixture.fee_rate,
            fixture.genesis_hash,
            AssetId::LIQUID_BTC,
        )
        .unwrap();

        assert_common_shape(&tx, &fixture, 4);
        let payment_hash = crate::utils::sha256_payment_hash(&fixture.preimage);
        assert_eq!(extract_preimage(&tx, &payment_hash), Some(fixture.preimage));
        // A witness item that does not hash to the swap's payment hash is not a preimage.
        assert_eq!(extract_preimage(&tx, &[0u8; 32]), None);
        let mut multi_input_tx = tx.clone();
        let mut unrelated_input = multi_input_tx.input[0].clone();
        unrelated_input.witness = TxInWitness::empty();
        multi_input_tx.input.insert(0, unrelated_input);
        assert_eq!(extract_preimage(&multi_input_tx, &payment_hash), None);
        assert_eq!(
            extract_preimage_at(&multi_input_tx, 1, &payment_hash),
            Some(fixture.preimage)
        );
        assert_eq!(
            tx.input[0].witness.script_witness[2],
            fixture.script.claim_script().as_bytes()
        );
        let sighash = SighashCache::new(&tx)
            .taproot_script_spend_signature_hash(
                0,
                &Prevouts::All(&[&fixture.prevout]),
                fixture.script.claim_leaf_hash(),
                SchnorrSighashType::Default,
                fixture.genesis_hash,
            )
            .unwrap()
            .to_byte_array();
        let claim_pubkey = XOnlyPublicKey::from_slice(
            &fixture
                .claim_key
                .x_only_public_key(crate::secp())
                .0
                .serialize(),
        )
        .unwrap();
        verify_signature(
            &tx.input[0].witness.script_witness[0],
            sighash,
            &claim_pubkey,
        );
    }

    #[test]
    fn csv_refund_sets_sequence_and_expected_witness() {
        let fixture = fixture();
        let tx = refund_by_csv(
            &single_input(&fixture),
            &blinding_key(&fixture),
            &fixture.script,
            &fixture.refund_key,
            &fixture.destination,
            fixture.fee_rate,
            fixture.genesis_hash,
            AssetId::LIQUID_BTC,
        )
        .unwrap();

        assert_common_shape(&tx, &fixture, 3);
        assert_eq!(
            tx.input[0].sequence,
            Sequence::from_consensus(fixture.script.csv_blocks())
        );
        assert_eq!(
            tx.input[0].witness.script_witness[1],
            fixture.script.refund_script().as_bytes()
        );
    }

    /// A three-input confidential sweep of same-address UTXOs: the destination
    /// receives the summed value minus fee, and the amount proofs verify against
    /// every prevout.
    #[test]
    fn batch_confidential_sweep_verifies() {
        let fixture = fixture();
        let values = [111_000u64, 90_000, 77_000];
        let inputs: Vec<(OutPoint, TxOut)> = values
            .iter()
            .enumerate()
            .map(|(i, &value)| {
                (
                    OutPoint::new(Txid::from_byte_array([i as u8 + 1; 32]), 0),
                    confidential_swap_output(&fixture, value),
                )
            })
            .collect();

        let tx = refund_by_csv(
            &inputs,
            &blinding_key(&fixture),
            &fixture.script,
            &fixture.refund_key,
            &fixture.destination,
            fixture.fee_rate,
            fixture.genesis_hash,
            AssetId::LIQUID_BTC,
        )
        .unwrap();

        assert_eq!(tx.input.len(), 3);
        assert_eq!(tx.output.len(), 2);

        let secp = Secp256k1::new();
        let prevouts: Vec<TxOut> = inputs.iter().map(|(_, p)| p.clone()).collect();
        tx.verify_tx_amt_proofs(&secp, &prevouts).unwrap();

        let fee = tx_fee(&tx).unwrap();
        let destination_secrets = tx.output[0]
            .unblind(&secp, fixture.destination_blinding_key)
            .unwrap();
        assert_eq!(destination_secrets.value, values.iter().sum::<u64>() - fee);
    }

    /// An opening as `elementsd` funds it: the swap output plus a confidential
    /// change output. The builders never produce this shape, so it is assembled
    /// from a claim to size `OPENING_VSIZE`.
    fn opening_shaped_tx(fixture: &Fixture, claim: &Transaction) -> Transaction {
        let mut tx = claim.clone();
        tx.output
            .insert(1, confidential_swap_output(fixture, 40_000));
        tx
    }

    #[test]
    fn quoted_vsizes_cover_the_built_transactions() {
        let fixture = fixture();
        let inputs = single_input(&fixture);
        let key = blinding_key(&fixture);
        let claim = claim_by_preimage(
            &inputs,
            &key,
            &fixture.script,
            &fixture.claim_key,
            &fixture.preimage,
            &fixture.destination,
            fixture.fee_rate,
            fixture.genesis_hash,
            AssetId::LIQUID_BTC,
        )
        .unwrap();
        let refund = refund_by_csv(
            &inputs,
            &key,
            &fixture.script,
            &fixture.refund_key,
            &fixture.destination,
            fixture.fee_rate,
            fixture.genesis_hash,
            AssetId::LIQUID_BTC,
        )
        .unwrap();
        let opening = opening_shaped_tx(&fixture, &claim);

        for (name, quoted, built) in [
            ("CLAIM_VSIZE", CLAIM_VSIZE, claim.discount_vsize() as u64),
            ("REFUND_VSIZE", REFUND_VSIZE, refund.discount_vsize() as u64),
            (
                "OPENING_VSIZE",
                OPENING_VSIZE,
                opening.discount_vsize() as u64,
            ),
        ] {
            assert!(quoted >= built, "{name} {quoted} under-quotes {built}");
            assert!(
                quoted <= built * 2,
                "{name} {quoted} over-quotes {built}; users pay this as base fee",
            );
        }
    }

    #[test]
    fn rejects_empty_inputs() {
        let fixture = fixture();
        let err = refund_by_csv(
            &[],
            &blinding_key(&fixture),
            &fixture.script,
            &fixture.refund_key,
            &fixture.destination,
            fixture.fee_rate,
            fixture.genesis_hash,
            AssetId::LIQUID_BTC,
        )
        .unwrap_err();
        assert!(matches!(err, TxError::NoInputs));
    }

    #[test]
    fn rejects_duplicate_inputs() {
        let fixture = fixture();
        let dup = (fixture.outpoint, fixture.prevout.clone());
        let err = refund_by_csv(
            &[dup.clone(), dup],
            &blinding_key(&fixture),
            &fixture.script,
            &fixture.refund_key,
            &fixture.destination,
            fixture.fee_rate,
            fixture.genesis_hash,
            AssetId::LIQUID_BTC,
        )
        .unwrap_err();
        assert!(matches!(err, TxError::DuplicateInput(_)));
    }

    #[test]
    fn rejects_wrong_asset_and_unconfidential_destination() {
        let fixture = fixture();
        let mut wrong_prevout = fixture.prevout.clone();
        wrong_prevout.asset = Asset::Explicit(AssetId::from_byte_array([10; 32]));
        let wrong_asset = spend_by_keypath(
            &fixture.claim_key,
            &fixture.refund_key,
            &fixture.destination,
            fixture.fee_rate,
            &[(fixture.outpoint, wrong_prevout)],
            &blinding_key(&fixture),
            fixture.script.clone(),
            fixture.genesis_hash,
            AssetId::LIQUID_BTC,
        )
        .unwrap_err();
        assert!(matches!(
            wrong_asset,
            TxError::UnsupportedLiquidAsset { .. }
        ));

        let unconfidential = Address {
            blinding_pubkey: None,
            ..fixture.destination.clone()
        };
        let missing_blinder = spend_by_keypath(
            &fixture.claim_key,
            &fixture.refund_key,
            &unconfidential,
            fixture.fee_rate,
            &single_input(&fixture),
            &blinding_key(&fixture),
            fixture.script.clone(),
            fixture.genesis_hash,
            AssetId::LIQUID_BTC,
        )
        .unwrap_err();
        assert!(matches!(missing_blinder, TxError::ConfidentialOutput(_)));
    }

    #[test]
    fn transaction_hex_roundtrip() {
        let fixture = fixture();
        let tx = refund_by_csv(
            &single_input(&fixture),
            &blinding_key(&fixture),
            &fixture.script,
            &fixture.refund_key,
            &fixture.destination,
            fixture.fee_rate,
            fixture.genesis_hash,
            AssetId::LIQUID_BTC,
        )
        .unwrap();
        assert_eq!(decode_tx_hex(&encode_tx_hex(&tx)).unwrap(), tx);
    }

    #[test]
    fn reconstructs_liquid_spend_input_from_opening_transaction() {
        let fixture = fixture();
        let tx = confidential_opening_tx(&fixture);
        let outpoint = OutPoint::new(tx.txid(), 0);
        let blinding_key =
            SecretKey::from_slice(&fixture.opening_blinding_key.secret_bytes()).unwrap();

        let (prevout, prevout_secrets) =
            crate::transaction::liquid_spend_input(&tx, outpoint, &blinding_key).unwrap();

        assert_eq!(prevout, tx.output[0]);
        assert_eq!(prevout_secrets.asset, fixture.secrets.asset);
        assert_eq!(prevout_secrets.value, fixture.secrets.value);

        // Validation is a separate step: it passes for the real script/asset and
        // rejects a foreign policy asset.
        crate::transaction::validate_liquid_prevout(
            &prevout,
            &prevout_secrets,
            &fixture.script,
            AssetId::LIQUID_BTC,
        )
        .unwrap();
        let wrong_asset = crate::transaction::validate_liquid_prevout(
            &prevout,
            &prevout_secrets,
            &fixture.script,
            AssetId::from_byte_array([9; 32]),
        )
        .unwrap_err();
        assert!(matches!(
            wrong_asset,
            TxError::UnsupportedLiquidAsset { .. }
        ));

        let wrong_outpoint = OutPoint::new(Txid::from_byte_array([99; 32]), 0);
        let error =
            crate::transaction::liquid_spend_input(&tx, wrong_outpoint, &blinding_key).unwrap_err();
        assert!(matches!(error, TxError::InvalidPrevout(_)));
    }

    fn confidential_opening_tx(fixture: &Fixture) -> Transaction {
        let secp = Secp256k1::new();
        let mut rng = bitcoin::key::rand::thread_rng();
        let (output, _, _, _) = TxOut::new_last_confidential(
            &mut rng,
            &secp,
            fixture.secrets.value,
            fixture.secrets.asset,
            fixture.script.script_pubkey(),
            fixture.script.blinding_pubkey(),
            &[fixture.secrets],
            &[],
        )
        .unwrap();
        Transaction {
            version: LIQUID_TX_VERSION,
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![output],
        }
    }

    #[test]
    fn common_build_interface_dispatches_all_liquid_paths() {
        use crate::transaction::{
            ClaimByKeypathParams, ClaimByPreimageParams, RefundByCsvParams, RefundByKeypathParams,
            build_claim_by_keypath, build_claim_by_preimage, build_refund_by_csv,
            build_refund_by_keypath,
        };

        let claim_fixture = fixture();
        let destination = claim_fixture.destination.to_string();
        let claim_key = claim_fixture.claim_key;
        let refund_key = claim_fixture.refund_key;
        let (inputs, blinding, script, genesis_hash) = spend_fields(&claim_fixture);
        let tx = decode_tx_hex(
            &build_claim_by_keypath(
                ClaimByKeypathParams::Liquid {
                    inputs,
                    blinding_key: blinding,
                    script,
                    genesis_hash,
                    policy_asset: AssetId::LIQUID_BTC,
                    claim_privkey: claim_key,
                    refund_privkey: refund_key,
                },
                &destination,
                1.0,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(tx.input[0].witness.script_witness.len(), 1);

        let refund_fixture = fixture();
        let destination = refund_fixture.destination.to_string();
        let claim_key = refund_fixture.claim_key;
        let refund_key = refund_fixture.refund_key;
        let (inputs, blinding, script, genesis_hash) = spend_fields(&refund_fixture);
        let tx = decode_tx_hex(
            &build_refund_by_keypath(
                RefundByKeypathParams::Liquid {
                    inputs,
                    blinding_key: blinding,
                    script,
                    genesis_hash,
                    policy_asset: AssetId::LIQUID_BTC,
                    claim_privkey: claim_key,
                    refund_privkey: refund_key,
                },
                &destination,
                1.0,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(tx.input[0].witness.script_witness.len(), 1);

        let preimage_fixture = fixture();
        let destination = preimage_fixture.destination.to_string();
        let claim_key = preimage_fixture.claim_key;
        let preimage = preimage_fixture.preimage;
        let (inputs, blinding, script, genesis_hash) = spend_fields(&preimage_fixture);
        let tx = decode_tx_hex(
            &build_claim_by_preimage(
                ClaimByPreimageParams::Liquid {
                    inputs,
                    blinding_key: blinding,
                    script,
                    genesis_hash,
                    policy_asset: AssetId::LIQUID_BTC,
                    claim_privkey: claim_key,
                    preimage,
                },
                &destination,
                1.0,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            extract_preimage(&tx, &crate::utils::sha256_payment_hash(&preimage)),
            Some(preimage)
        );

        let csv_fixture = fixture();
        let destination = csv_fixture.destination.to_string();
        let refund_key = csv_fixture.refund_key;
        let expected_sequence = Sequence::from_consensus(csv_fixture.script.csv_blocks());
        let (inputs, blinding, script, genesis_hash) = spend_fields(&csv_fixture);
        let tx = decode_tx_hex(
            &build_refund_by_csv(
                RefundByCsvParams::Liquid {
                    inputs,
                    blinding_key: blinding,
                    script,
                    genesis_hash,
                    policy_asset: AssetId::LIQUID_BTC,
                    refund_privkey: refund_key,
                },
                &destination,
                1.0,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(tx.input[0].sequence, expected_sequence);
    }

    fn spend_fields(
        fixture: &Fixture,
    ) -> (
        Vec<(OutPoint, TxOut)>,
        SecretKey,
        LiquidSwapScript,
        BlockHash,
    ) {
        (
            single_input(fixture),
            blinding_key(fixture),
            fixture.script.clone(),
            fixture.genesis_hash,
        )
    }
}
