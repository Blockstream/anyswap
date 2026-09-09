//! Bitcoin transaction builders for the [`BitcoinSwapScript`] spending paths.
//!
//! Each function constructs a transaction that spends one or more UTXOs locked by
//! the same [`BitcoinSwapScript`] to a single destination output, deducting the
//! fee from the swept value. A single input is just the one-element case; passing
//! several sweeps them all in one transaction (for example, refunding a swap's
//! deposit together with any stray deposits at the same address).

use std::{collections::HashSet, str::FromStr};

use bitcoin::{
    Address, Amount, FeeRate, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
    absolute,
    hashes::Hash,
    secp256k1::{Keypair, Message, SecretKey},
    sighash::{Prevouts, SighashCache, TapSighashType},
    taproot::{LeafVersion, TapLeafHash},
    transaction,
};

use super::TxError;
use crate::{musig, script::BitcoinSwapScript};

/// Worst-case Schnorr signature: 64 bytes (SIGHASH_DEFAULT).
const DUMMY_SCHNORR_SIG: [u8; 64] = [0; 64];

/// Dummy preimage placeholder.
const DUMMY_PREIMAGE: [u8; 32] = [0; 32];

/// Cooperative keypath spend (claim or refund), the cheaper happy path; the
/// vsize of a single-input, single-output transaction.
pub const KEYPATH_VSIZE: u64 = 111;

/// Script-path claim that reveals the preimage; the vsize `base_fee` quotes.
pub const CLAIM_VSIZE: u64 = 152;

/// Script-path CSV refund.
pub const REFUND_VSIZE: u64 = 138;

/// Opening tx: one funding input plus change.
pub const OPENING_VSIZE: u64 = 154;

/// Cooperative MuSig2 keypath spend, used for both `claim_by_keypath` and
/// `refund_by_keypath`. Once the counterparty has revealed its half, the
/// caller holds both privkeys and signs locally; each input's witness is just
/// the 64-byte Schnorr signature over the tweaked output key. All inputs must be
/// UTXOs of the same `script`.
pub fn spend_by_keypath(
    inputs: &[(OutPoint, Amount)],
    script: &BitcoinSwapScript,
    claim_privkey: &SecretKey,
    refund_privkey: &SecretKey,
    destination: &ScriptBuf,
    fee_rate: FeeRate,
) -> Result<Transaction, TxError> {
    let prevouts = make_prevouts(inputs, script);

    build_with_fee(fee_rate, |fee, estimate| {
        let mut tx = build_tx(inputs, destination, fee, Sequence::ZERO)?;
        if estimate {
            for txin in &mut tx.input {
                txin.witness.push(DUMMY_SCHNORR_SIG);
            }
        } else {
            let sighashes = keypath_sighashes(&tx, &prevouts)?;
            for (i, sighash) in sighashes.iter().enumerate() {
                let sig = musig::sign_keypath(
                    *claim_privkey,
                    *refund_privkey,
                    Some(script.merkle_root()),
                    sighash,
                )?;
                tx.input[i].witness.push(sig);
            }
        }
        Ok(tx)
    })
}

/// Script-path claim revealing the preimage. Per-input witness:
/// `[sig, preimage, script, control_block]`. All inputs share the one preimage
/// and must be UTXOs of the same `script`.
pub fn claim_by_preimage(
    inputs: &[(OutPoint, Amount)],
    script: &BitcoinSwapScript,
    claim_privkey: &SecretKey,
    preimage: &[u8; 32],
    destination: &ScriptBuf,
    fee_rate: FeeRate,
) -> Result<Transaction, TxError> {
    let prevouts = make_prevouts(inputs, script);
    let leaf_hash = TapLeafHash::from_script(script.claim_script(), LeafVersion::TapScript);
    let control_block = script.claim_control_block().serialize();

    build_with_fee(fee_rate, |fee, estimate| {
        let mut tx = build_tx(inputs, destination, fee, Sequence::ZERO)?;
        let (sigs, pre): (Vec<Vec<u8>>, &[u8]) = if estimate {
            (
                vec![DUMMY_SCHNORR_SIG.to_vec(); inputs.len()],
                &DUMMY_PREIMAGE,
            )
        } else {
            let sighashes = script_path_sighashes(&tx, &prevouts, leaf_hash)?;
            let sigs = sighashes
                .iter()
                .map(|sh| schnorr_sign(sh, claim_privkey))
                .collect();
            (sigs, preimage)
        };
        for (i, sig) in sigs.into_iter().enumerate() {
            let w = &mut tx.input[i].witness;
            w.push(sig);
            w.push(pre);
            w.push(script.claim_script().as_bytes());
            w.push(&control_block);
        }
        Ok(tx)
    })
}

/// Script-path refund after the CSV timeout. Per-input witness: `[sig, script,
/// control_block]`. The CSV value is read from the [`BitcoinSwapScript`] so the
/// `nSequence` always matches what the refund leaf commits to; every input must
/// have matured to that depth for the transaction to be final. All inputs must
/// be UTXOs of the same `script`.
pub fn refund_by_csv(
    inputs: &[(OutPoint, Amount)],
    script: &BitcoinSwapScript,
    refund_privkey: &SecretKey,
    destination: &ScriptBuf,
    fee_rate: FeeRate,
) -> Result<Transaction, TxError> {
    let prevouts = make_prevouts(inputs, script);
    let leaf_hash = TapLeafHash::from_script(script.refund_script(), LeafVersion::TapScript);
    let control_block = script.refund_control_block().serialize();
    let sequence = Sequence::from_consensus(script.csv_blocks());

    build_with_fee(fee_rate, |fee, estimate| {
        let mut tx = build_tx(inputs, destination, fee, sequence)?;
        let sigs: Vec<Vec<u8>> = if estimate {
            vec![DUMMY_SCHNORR_SIG.to_vec(); inputs.len()]
        } else {
            let sighashes = script_path_sighashes(&tx, &prevouts, leaf_hash)?;
            sighashes
                .iter()
                .map(|sh| schnorr_sign(sh, refund_privkey))
                .collect()
        };
        for (i, sig) in sigs.into_iter().enumerate() {
            let w = &mut tx.input[i].witness;
            w.push(sig);
            w.push(script.refund_script().as_bytes());
            w.push(&control_block);
        }
        Ok(tx)
    })
}

/// Two-pass fee estimator. The first pass uses `estimate=true` (caller should use
/// dummy constants instead of real data), the second pass uses `estimate=false`.
fn build_with_fee(
    fee_rate: FeeRate,
    build: impl Fn(u64, bool) -> Result<Transaction, TxError>,
) -> Result<Transaction, TxError> {
    let tx = build(0, true)?;
    let fee = fee_rate
        .checked_mul_by_weight(tx.weight())
        .map(|a| a.to_sat())
        .ok_or(TxError::FeeOverflow)?;
    build(fee, false)
}

/// Taproot key-spend sighashes, one per input, each committing to every prevout.
fn keypath_sighashes(tx: &Transaction, prevouts: &[TxOut]) -> Result<Vec<[u8; 32]>, TxError> {
    let mut cache = SighashCache::new(tx);
    (0..prevouts.len())
        .map(|i| {
            cache
                .taproot_key_spend_signature_hash(
                    i,
                    &Prevouts::All(prevouts),
                    TapSighashType::Default,
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
) -> Result<Vec<[u8; 32]>, TxError> {
    let mut cache = SighashCache::new(tx);
    (0..prevouts.len())
        .map(|i| {
            cache
                .taproot_script_spend_signature_hash(
                    i,
                    &Prevouts::All(prevouts),
                    leaf_hash,
                    TapSighashType::Default,
                )
                .map(|sh| sh.to_byte_array())
                .map_err(|e| TxError::InvalidSighash(e.to_string()))
        })
        .collect()
}

fn schnorr_sign(sighash: &[u8; 32], privkey: &SecretKey) -> Vec<u8> {
    let secp = crate::secp();
    let keypair = Keypair::from_secret_key(secp, privkey);
    let msg = Message::from_digest(*sighash);
    secp.sign_schnorr(&msg, &keypair).as_ref().to_vec()
}

fn make_prevout(value: Amount, script: &BitcoinSwapScript) -> TxOut {
    TxOut {
        value,
        script_pubkey: script.script_pubkey(),
    }
}

fn make_prevouts(inputs: &[(OutPoint, Amount)], script: &BitcoinSwapScript) -> Vec<TxOut> {
    inputs
        .iter()
        .map(|(_, value)| make_prevout(*value, script))
        .collect()
}

/// Builds the unsigned skeleton: one input per `(outpoint, value)` and a single
/// output paying the summed value minus `fee`. Rejects an empty input set,
/// duplicate inputs, and any fee that would leave a below-dust (or negative)
/// output.
fn build_tx(
    inputs: &[(OutPoint, Amount)],
    destination: &ScriptBuf,
    fee: u64,
    sequence: Sequence,
) -> Result<Transaction, TxError> {
    if inputs.is_empty() {
        return Err(TxError::NoInputs);
    }
    if inputs.len() > 1 {
        let mut seen = HashSet::with_capacity(inputs.len());
        for (outpoint, _) in inputs {
            if !seen.insert(outpoint) {
                return Err(TxError::DuplicateInput(outpoint.to_string()));
            }
        }
    }
    let total_in = inputs
        .iter()
        .try_fold(Amount::ZERO, |acc, (_, v)| acc.checked_add(*v))
        .ok_or(TxError::InputOverflow)?;
    let value = total_in
        .checked_sub(Amount::from_sat(fee))
        .ok_or(TxError::InsufficientValue {
            input_sat: total_in.to_sat(),
            fee_sat: fee,
        })?;
    let dust = destination.minimal_non_dust();
    if value < dust {
        return Err(TxError::DustOutput {
            output_sat: value.to_sat(),
            minimum_sat: dust.to_sat(),
        });
    }
    Ok(Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: inputs
            .iter()
            .map(|(outpoint, _)| TxIn {
                previous_output: *outpoint,
                script_sig: ScriptBuf::new(),
                sequence,
                witness: Witness::new(),
            })
            .collect(),
        output: vec![TxOut {
            value,
            script_pubkey: destination.clone(),
        }],
    })
}

/// Extracts a 32-byte preimage from a claim-by-preimage witness.
/// Witness layout: `[sig, preimage, claim_script, control_block]`.
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
        .nth(1)?
        .try_into()
        .ok()?;
    (crate::utils::sha256_payment_hash(&preimage) == *payment_hash).then_some(preimage)
}

pub fn encode_tx_hex(tx: &Transaction) -> String {
    hex::encode(bitcoin::consensus::serialize(tx))
}

pub fn decode_tx_hex(tx_hex: &str) -> Result<Transaction, TxError> {
    let bytes = hex::decode(tx_hex).map_err(|e| TxError::Decode(e.to_string()))?;
    bitcoin::consensus::deserialize(&bytes).map_err(|e| TxError::Decode(e.to_string()))
}

/// Resolves the funded value of the opening output at `vout`, checking it pays
/// `script`. The caller compares the value against the agreed amount. Bitcoin
/// values are explicit, so no unblinding is needed.
pub fn opening_value(
    tx: &Transaction,
    vout: u32,
    script: &BitcoinSwapScript,
) -> Result<Amount, TxError> {
    let output = tx
        .output
        .get(vout as usize)
        .ok_or_else(|| TxError::InvalidPrevout(format!("vout {vout} out of range")))?;
    if output.script_pubkey != script.script_pubkey() {
        return Err(TxError::InvalidPrevout("script_pubkey mismatch".into()));
    }
    Ok(output.value)
}

/// Miner fee of a signed tx: `input_value - Σ outputs`. Bitcoin only (Liquid
/// carries an explicit fee output). `input_value` is the summed value of every
/// input the tx spends.
pub(crate) fn bitcoin_tx_fee(tx: &Transaction, input_value: u64) -> Result<u64, TxError> {
    let output_value = tx
        .output
        .iter()
        .try_fold(0_u64, |total, out| total.checked_add(out.value.to_sat()))
        .ok_or(TxError::FeeOverflow)?;
    input_value
        .checked_sub(output_value)
        .ok_or(TxError::InsufficientValue {
            input_sat: input_value,
            fee_sat: output_value,
        })
}

pub(crate) fn parse_destination(destination: &str) -> Result<ScriptBuf, TxError> {
    let address =
        Address::from_str(destination).map_err(|e| TxError::InvalidDestination(e.to_string()))?;
    Ok(address.assume_checked().script_pubkey())
}

/// A vbyte is four weight units and [`FeeRate`] counts sats per 1000 weight
/// units, so one sat/vB is 250 sat/kwu.
const SAT_PER_KWU_PER_SAT_PER_VB: f64 = 250.0;

/// Converts a sat/vB rate for the Bitcoin builders, going through sat/kwu so a
/// fractional rate survives: `FeeRate::from_sat_per_vb` only takes whole sats.
/// The relay floor is separate and lives in the chain client.
pub fn sat_per_vb(fee_rate: f64) -> Result<FeeRate, TxError> {
    if !fee_rate.is_finite() || fee_rate < 0.0 {
        return Err(TxError::InvalidFeeRate(fee_rate));
    }
    let sat_per_kwu = (fee_rate * SAT_PER_KWU_PER_SAT_PER_VB).ceil();
    if sat_per_kwu > u64::MAX as f64 {
        return Err(TxError::InvalidFeeRate(fee_rate));
    }
    Ok(FeeRate::from_sat_per_kwu(sat_per_kwu as u64))
}

#[cfg(test)]
mod tests {
    use bitcoin::{Txid, hashes::sha256, secp256k1::XOnlyPublicKey};

    use super::*;

    struct Fixture {
        script: BitcoinSwapScript,
        outpoint: OutPoint,
        input_value: Amount,
        destination: ScriptBuf,
        fee_rate: FeeRate,
        claim_sk: SecretKey,
        refund_sk: SecretKey,
        preimage: [u8; 32],
    }

    fn keypair(seed: [u8; 32]) -> (SecretKey, XOnlyPublicKey) {
        let sk = SecretKey::from_slice(&seed).unwrap();
        let xonly = sk.x_only_public_key(crate::secp()).0;
        (sk, xonly)
    }

    fn fixture() -> Fixture {
        let (claim_sk, claim_pk) = keypair([1; 32]);
        let (refund_sk, refund_pk) = keypair([2; 32]);
        let preimage = [42u8; 32];
        let payment_hash = sha256::Hash::hash(&preimage).to_byte_array();
        let csv_blocks = 144;
        let script = BitcoinSwapScript::new(claim_pk, refund_pk, payment_hash, csv_blocks).unwrap();

        let dummy_xonly = SecretKey::from_slice(&[5; 32])
            .unwrap()
            .x_only_public_key(crate::secp())
            .0;
        let destination = ScriptBuf::new_p2tr(crate::secp(), dummy_xonly, None);

        Fixture {
            script,
            outpoint: OutPoint {
                txid: Txid::from_byte_array([7; 32]),
                vout: 0,
            },
            input_value: Amount::from_sat(100_000),
            destination,
            fee_rate: FeeRate::from_sat_per_vb(10).unwrap(),
            claim_sk,
            refund_sk,
            preimage,
        }
    }

    /// `n` distinct swap-address UTXOs, all worth `input_value`.
    fn inputs(f: &Fixture, n: u32) -> Vec<(OutPoint, Amount)> {
        (0..n)
            .map(|vout| {
                (
                    OutPoint {
                        txid: f.outpoint.txid,
                        vout,
                    },
                    f.input_value,
                )
            })
            .collect()
    }

    fn assert_spends(tx: &Transaction, f: &Fixture) {
        let prevout = make_prevout(f.input_value, &f.script);
        tx.verify(|outpoint| {
            tx.input
                .iter()
                .any(|i| i.previous_output == *outpoint)
                .then(|| prevout.clone())
        })
        .expect("spending tx must verify against the swap output(s)");
    }

    #[test]
    fn spend_by_keypath_spends() {
        let f = fixture();
        let tx = spend_by_keypath(
            &inputs(&f, 1),
            &f.script,
            &f.claim_sk,
            &f.refund_sk,
            &f.destination,
            f.fee_rate,
        )
        .unwrap();
        assert_spends(&tx, &f);
    }

    #[test]
    fn claim_by_preimage_spends() {
        let f = fixture();
        let tx = claim_by_preimage(
            &inputs(&f, 1),
            &f.script,
            &f.claim_sk,
            &f.preimage,
            &f.destination,
            f.fee_rate,
        )
        .unwrap();
        assert_spends(&tx, &f);
        let payment_hash = crate::utils::sha256_payment_hash(&f.preimage);
        assert_eq!(extract_preimage(&tx, &payment_hash), Some(f.preimage));
        // A witness item that does not hash to the swap's payment hash is not a preimage.
        assert_eq!(extract_preimage(&tx, &[0u8; 32]), None);

        let mut multi_input_tx = tx;
        let mut unrelated_input = multi_input_tx.input[0].clone();
        unrelated_input.witness = bitcoin::Witness::default();
        multi_input_tx.input.insert(0, unrelated_input);
        assert_eq!(extract_preimage(&multi_input_tx, &payment_hash), None);
        assert_eq!(
            extract_preimage_at(&multi_input_tx, 1, &payment_hash),
            Some(f.preimage)
        );
    }

    #[test]
    fn refund_by_csv_spends() {
        let f = fixture();
        let tx = refund_by_csv(
            &inputs(&f, 1),
            &f.script,
            &f.refund_sk,
            &f.destination,
            f.fee_rate,
        )
        .unwrap();
        assert_spends(&tx, &f);
    }

    /// Multi-input sweeps: every input signs over the full prevout set, so the
    /// whole transaction must still verify.
    #[test]
    fn batch_sweeps_verify() {
        let f = fixture();

        let keypath = spend_by_keypath(
            &inputs(&f, 3),
            &f.script,
            &f.claim_sk,
            &f.refund_sk,
            &f.destination,
            f.fee_rate,
        )
        .unwrap();
        assert_eq!(keypath.input.len(), 3);
        assert_spends(&keypath, &f);

        let preimage = claim_by_preimage(
            &inputs(&f, 3),
            &f.script,
            &f.claim_sk,
            &f.preimage,
            &f.destination,
            f.fee_rate,
        )
        .unwrap();
        assert_eq!(preimage.input.len(), 3);
        assert_spends(&preimage, &f);

        let csv = refund_by_csv(
            &inputs(&f, 3),
            &f.script,
            &f.refund_sk,
            &f.destination,
            f.fee_rate,
        )
        .unwrap();
        assert_eq!(csv.input.len(), 3);
        assert_spends(&csv, &f);
    }

    fn opening_tx(f: &Fixture, num_inputs: usize) -> Transaction {
        let input = (0..num_inputs)
            .map(|i| {
                let mut witness = Witness::new();
                witness.push(DUMMY_SCHNORR_SIG);
                TxIn {
                    previous_output: OutPoint {
                        txid: Txid::from_byte_array([i as u8; 32]),
                        vout: 0,
                    },
                    script_sig: ScriptBuf::new(),
                    sequence: Sequence::ZERO,
                    witness,
                }
            })
            .collect();
        Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input,
            output: vec![
                TxOut {
                    value: f.input_value,
                    script_pubkey: f.script.script_pubkey(),
                },
                TxOut {
                    value: f.input_value,
                    script_pubkey: f.destination.clone(),
                },
            ],
        }
    }

    #[test]
    fn swap_tx_vsizes() {
        let f = fixture();

        let keypath = spend_by_keypath(
            &inputs(&f, 1),
            &f.script,
            &f.claim_sk,
            &f.refund_sk,
            &f.destination,
            f.fee_rate,
        )
        .unwrap();
        let preimage_claim = claim_by_preimage(
            &inputs(&f, 1),
            &f.script,
            &f.claim_sk,
            &f.preimage,
            &f.destination,
            f.fee_rate,
        )
        .unwrap();
        let csv_refund = refund_by_csv(
            &inputs(&f, 1),
            &f.script,
            &f.refund_sk,
            &f.destination,
            f.fee_rate,
        )
        .unwrap();

        assert_eq!(keypath.vsize(), KEYPATH_VSIZE as usize);
        assert_eq!(preimage_claim.vsize(), CLAIM_VSIZE as usize);
        assert_eq!(csv_refund.vsize(), REFUND_VSIZE as usize);

        assert_eq!(opening_tx(&f, 1).vsize(), OPENING_VSIZE as usize);
        assert_eq!(opening_tx(&f, 2).vsize(), 212);
        assert_eq!(opening_tx(&f, 3).vsize(), 269);
    }

    #[test]
    fn base_fee_covers_what_a_claim_actually_pays() {
        let f = fixture();
        let rate = 1.5;
        let claim = claim_by_preimage(
            &inputs(&f, 1),
            &f.script,
            &f.claim_sk,
            &f.preimage,
            &f.destination,
            sat_per_vb(rate).unwrap(),
        )
        .unwrap();

        let paid = bitcoin_tx_fee(&claim, f.input_value.to_sat()).unwrap();
        let quoted = crate::fee::bitcoin::base_fee(crate::types::SwapType::SwapIn, rate, 1.0);

        assert!(paid <= quoted, "claim pays {paid}, quote is {quoted}");
    }

    #[test]
    fn rejects_empty_inputs() {
        let f = fixture();
        let err =
            refund_by_csv(&[], &f.script, &f.refund_sk, &f.destination, f.fee_rate).unwrap_err();
        assert!(matches!(err, TxError::NoInputs));
    }

    #[test]
    fn rejects_duplicate_inputs() {
        let f = fixture();
        let dup = (f.outpoint, f.input_value);
        let err = refund_by_csv(
            &[dup, dup],
            &f.script,
            &f.refund_sk,
            &f.destination,
            f.fee_rate,
        )
        .unwrap_err();
        assert!(matches!(err, TxError::DuplicateInput(_)));
    }

    #[test]
    fn refund_by_csv_rejects_fee_above_input() {
        let f = fixture();
        // Above the dust limit but below the single-input fee, so the fee, not the
        // dust floor, is what rejects it.
        let tiny_input = Amount::from_sat(500);
        let err = refund_by_csv(
            &[(f.outpoint, tiny_input)],
            &f.script,
            &f.refund_sk,
            &f.destination,
            f.fee_rate,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            TxError::InsufficientValue { input_sat: 500, .. },
        ));
    }

    #[test]
    fn rejects_dust_output() {
        let f = fixture();
        // Covers the fee (~1380 sat at 10 sat/vB) but leaves a sub-dust output.
        let input = Amount::from_sat(1480);
        let err = refund_by_csv(
            &[(f.outpoint, input)],
            &f.script,
            &f.refund_sk,
            &f.destination,
            f.fee_rate,
        )
        .unwrap_err();
        assert!(matches!(err, TxError::DustOutput { .. }));
    }
}
