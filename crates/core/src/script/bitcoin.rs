//! Locks the on-chain side of the swap in a P2TR output. The cooperative path
//! is a MuSig2 keypath spend, used for both claim and refund. Two script-path
//! leaves provide unilateral fallbacks if the counterparty doesn't cooperate:
//! claim by revealing the preimage, or refund after a CSV timeout.

use anyhow::Error;
use bitcoin::{
    Address, ScriptBuf, XOnlyPublicKey,
    blockdata::{opcodes::all::*, script::Builder},
    hashes::{Hash, ripemd160},
    key::TweakedPublicKey,
    taproot::{ControlBlock, LeafVersion, TapNodeHash, TaprootBuilder},
};

use super::validate_parameters;
use crate::types::SwapNetwork;

/// P2TR output with MuSig2 internal key and two leaf scripts.
#[derive(Debug, Clone)]
pub struct BitcoinSwapScript {
    internal_key: XOnlyPublicKey,
    output_key: TweakedPublicKey,
    merkle_root: TapNodeHash,
    csv_blocks: u32,
    claim_script: ScriptBuf,
    refund_script: ScriptBuf,
    claim_control_block: ControlBlock,
    refund_control_block: ControlBlock,
}

impl BitcoinSwapScript {
    /// `payment_hash` is `SHA256(preimage)`; the claim leaf stores its `RIPEMD160`
    /// form so the witness preimage matches via `OP_HASH160`.
    ///
    /// `csv_blocks` must be in `1..=0xFFFF` so the resulting `nSequence` is a valid
    /// block-based relative timelock with the disable bit unset.
    pub fn new(
        claim_pubkey: XOnlyPublicKey,
        refund_pubkey: XOnlyPublicKey,
        payment_hash: [u8; 32],
        csv_blocks: u32,
    ) -> Result<Self, Error> {
        let claim_script = build_claim_script(&claim_pubkey, &payment_hash);
        let refund_script = build_refund_script(&refund_pubkey, csv_blocks);
        let internal_key = validate_parameters(claim_pubkey, refund_pubkey, csv_blocks)?;

        let spend_info = TaprootBuilder::new()
            .add_leaf(1, claim_script.clone())
            .map_err(|_| Error::msg("taproot build failed"))?
            .add_leaf(1, refund_script.clone())
            .map_err(|_| Error::msg("taproot build failed"))?
            .finalize(crate::secp(), internal_key)
            .map_err(|_| Error::msg("taproot build failed"))?;

        let merkle_root = spend_info
            .merkle_root()
            .ok_or(Error::msg("taproot build failed"))?;
        let claim_control_block = spend_info
            .control_block(&(claim_script.clone(), LeafVersion::TapScript))
            .ok_or(Error::msg("taproot build failed"))?;
        let refund_control_block = spend_info
            .control_block(&(refund_script.clone(), LeafVersion::TapScript))
            .ok_or(Error::msg("taproot build failed"))?;

        Ok(Self {
            internal_key,
            output_key: spend_info.output_key(),
            merkle_root,
            csv_blocks,
            claim_script,
            refund_script,
            claim_control_block,
            refund_control_block,
        })
    }

    /// Untweaked MuSig2 aggregate of the two participant pubkeys.
    pub fn internal_key(&self) -> XOnlyPublicKey {
        self.internal_key
    }

    /// Block count committed in the refund leaf; also the required `nSequence`
    /// when spending via that leaf.
    pub fn csv_blocks(&self) -> u32 {
        self.csv_blocks
    }

    /// Tweaked taproot output key.
    pub fn output_key(&self) -> TweakedPublicKey {
        self.output_key
    }

    /// Tap tree merkle root. Fed to [`musig::sign_keypath`] for keypath spends.
    pub fn merkle_root(&self) -> TapNodeHash {
        self.merkle_root
    }

    /// Leaf script for the claim-by-preimage path.
    pub fn claim_script(&self) -> &ScriptBuf {
        &self.claim_script
    }

    /// Leaf script for the refund-by-csv path.
    pub fn refund_script(&self) -> &ScriptBuf {
        &self.refund_script
    }

    /// Tap-script control block for the claim leaf, used in the script-path witness.
    pub fn claim_control_block(&self) -> &ControlBlock {
        &self.claim_control_block
    }

    /// Tap-script control block for the refund leaf, used in the script-path witness.
    pub fn refund_control_block(&self) -> &ControlBlock {
        &self.refund_control_block
    }

    /// P2TR `scriptPubKey` for the opening output.
    pub fn script_pubkey(&self) -> ScriptBuf {
        ScriptBuf::new_p2tr_tweaked(self.output_key)
    }
}

impl BitcoinSwapScript {
    /// P2TR address for the opening output.
    pub fn address(&self, network: SwapNetwork) -> String {
        Address::p2tr_tweaked(self.output_key, network.bitcoin_network()).to_string()
    }
}

/// `OP_SIZE <32> OP_EQUALVERIFY OP_HASH160 <RIPEMD160(H)> OP_EQUALVERIFY <A> OP_CHECKSIG`
///
/// The size check keeps the leaf spendable only by a preimage Lightning can carry:
/// the payer picks `H`, and a shorter one would take the output while the HTLC
/// stays unsettled.
fn build_claim_script(claim_pubkey: &XOnlyPublicKey, payment_hash: &[u8; 32]) -> ScriptBuf {
    let ripemd = ripemd160::Hash::hash(payment_hash);
    Builder::new()
        .push_opcode(OP_SIZE)
        .push_int(32)
        .push_opcode(OP_EQUALVERIFY)
        .push_opcode(OP_HASH160)
        .push_slice(ripemd.as_byte_array())
        .push_opcode(OP_EQUALVERIFY)
        .push_x_only_key(claim_pubkey)
        .push_opcode(OP_CHECKSIG)
        .into_script()
}

/// `<B> OP_CHECKSIGVERIFY <N> OP_CHECKSEQUENCEVERIFY`
fn build_refund_script(refund_pubkey: &XOnlyPublicKey, csv_blocks: u32) -> ScriptBuf {
    Builder::new()
        .push_x_only_key(refund_pubkey)
        .push_opcode(OP_CHECKSIGVERIFY)
        .push_int(csv_blocks as i64)
        .push_opcode(OP_CSV)
        .into_script()
}

#[cfg(test)]
mod tests {
    use bitcoin::{Opcode, blockdata::script::Instruction, hashes::sha256, secp256k1::SecretKey};

    use super::*;
    use crate::musig;

    struct Fixture {
        claim_pk: XOnlyPublicKey,
        refund_pk: XOnlyPublicKey,
        payment_hash: [u8; 32],
        csv_blocks: u32,
        script: BitcoinSwapScript,
    }

    fn xonly(seed: [u8; 32]) -> XOnlyPublicKey {
        SecretKey::from_slice(&seed)
            .unwrap()
            .x_only_public_key(crate::secp())
            .0
    }

    fn opcodes(script: &ScriptBuf) -> Vec<Opcode> {
        script
            .instructions()
            .filter_map(|ins| match ins.unwrap() {
                Instruction::Op(op) => Some(op),
                Instruction::PushBytes(_) => None,
            })
            .collect()
    }

    fn fixture() -> Fixture {
        let claim_pk = xonly([1; 32]);
        let refund_pk = xonly([2; 32]);
        let payment_hash = sha256::Hash::hash(&[42u8; 32]).to_byte_array();
        let csv_blocks = 144;
        let script = BitcoinSwapScript::new(claim_pk, refund_pk, payment_hash, csv_blocks).unwrap();

        Fixture {
            claim_pk,
            refund_pk,
            payment_hash,
            csv_blocks,
            script,
        }
    }

    #[test]
    fn rejects_duplicate_pubkeys() {
        let f = fixture();
        let err = BitcoinSwapScript::new(f.claim_pk, f.claim_pk, f.payment_hash, f.csv_blocks)
            .unwrap_err();
        assert_eq!(err.to_string(), "claim and refund pubkeys must differ");
    }

    #[test]
    fn rejects_out_of_range_csv() {
        let f = fixture();
        for bad in [0u32, 0x10000] {
            let err =
                BitcoinSwapScript::new(f.claim_pk, f.refund_pk, f.payment_hash, bad).unwrap_err();
            assert_eq!(
                err.to_string(),
                format!("csv_blocks must be in 1..=65535, got {bad}"),
            );
        }
    }

    #[test]
    fn address_is_deterministic() {
        let f = fixture();
        let other =
            BitcoinSwapScript::new(f.claim_pk, f.refund_pk, f.payment_hash, f.csv_blocks).unwrap();
        assert_eq!(
            f.script.address(SwapNetwork::Regtest).to_string(),
            other.address(SwapNetwork::Regtest).to_string(),
        );
    }

    #[test]
    fn internal_key_matches_musig_aggregate() {
        let f = fixture();
        let expected = musig::aggregate_xonly(f.claim_pk, f.refund_pk).unwrap();
        assert_eq!(f.script.internal_key(), expected);
    }

    #[test]
    fn claim_leaf_structure() {
        let f = fixture();
        assert_eq!(
            opcodes(f.script.claim_script()),
            vec![
                OP_SIZE,
                OP_EQUALVERIFY,
                OP_HASH160,
                OP_EQUALVERIFY,
                OP_CHECKSIG
            ],
        );
    }

    #[test]
    fn refund_leaf_structure() {
        let f = fixture();
        assert_eq!(
            opcodes(f.script.refund_script()),
            vec![OP_CHECKSIGVERIFY, OP_CSV],
        );
    }
}
