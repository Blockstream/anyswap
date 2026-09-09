//! Locks the Liquid side of the swap in a confidential P2TR output. The
//! cooperative path is a MuSig2 keypath spend. Two script-path leaves provide
//! unilateral claim-by-preimage and refund-after-CSV fallbacks.

use anyhow::Error;
use bitcoin::{
    XOnlyPublicKey,
    hashes::{Hash, ripemd160},
    secp256k1::{PublicKey, SecretKey, ecdh::SharedSecret},
};
use elements::{
    Address, Script,
    opcodes::all::*,
    schnorr::{TweakedPublicKey, XOnlyPublicKey as LiquidXOnlyPublicKey},
    script::Builder,
    secp256k1_zkp::{
        PublicKey as BlindingPublicKey, Secp256k1 as LiquidSecp256k1, SecretKey as LiquidSecretKey,
    },
    taproot::{ControlBlock, LeafVersion, TapLeafHash, TapNodeHash, TapTweakHash, TaprootBuilder},
};

use super::validate_parameters;
use crate::types::SwapNetwork;

/// Derives the shared ECDH blinding key and its Elements public key.
/// Why do we need the public key of common secret? The problem source is
/// liquid blinding process. Instead of using blinding key it generates the
/// ephemeral key for each transaction and use it for blinding. Using the both-parties-known
/// pub key and private key allows to unblind the transaction and spend it if we need.
///
/// Plugin computes the common secret: k = s * R = r * S = r * s * G; K = k*G (that pubkey is used
/// on the address) Elements generates: e <-$; E = e * G
/// Elements computes common secret: B = e * K = E * K
/// Final tx nonce is E, that allows both sender and receiver computes common secret and unblind tx
pub fn derive_liquid_blinding_key(
    own_privkey: &SecretKey,
    peer_pubkey: &PublicKey,
) -> Result<(BlindingPublicKey, SecretKey), String> {
    let shared = SharedSecret::new(peer_pubkey, own_privkey);
    let blinding_key = SecretKey::from_slice(&shared.secret_bytes())
        .map_err(|error| format!("invalid shared blinding key: {error}"))?;
    let liquid_secret = LiquidSecretKey::from_slice(&blinding_key.secret_bytes())
        .map_err(|error| format!("invalid Elements blinding key: {error}"))?;
    let blinding_pubkey =
        BlindingPublicKey::from_secret_key(&LiquidSecp256k1::new(), &liquid_secret);
    Ok((blinding_pubkey, blinding_key))
}

/// Liquid P2TR output with a MuSig2 internal key and two leaf scripts.
#[derive(Clone, Debug)]
pub struct LiquidSwapScript {
    internal_key: XOnlyPublicKey,
    output_key: TweakedPublicKey,
    tap_tweak: TapTweakHash,
    merkle_root: TapNodeHash,
    csv_blocks: u32,
    claim_script: Script,
    refund_script: Script,
    claim_leaf_hash: TapLeafHash,
    refund_leaf_hash: TapLeafHash,
    claim_control_block: ControlBlock,
    refund_control_block: ControlBlock,
    blinding_pubkey: BlindingPublicKey,
}

impl LiquidSwapScript {
    /// Builds the Liquid taproot commitment and associates it with the public
    /// blinding key used to produce its confidential deposit address.
    ///
    /// `payment_hash` is `SHA256(preimage)`; the claim leaf stores its
    /// `RIPEMD160` form so the witness preimage matches via `OP_HASH160`.
    ///
    /// `csv_blocks` must be in `1..=0xFFFF` so the resulting `nSequence` is a
    /// valid block-based relative timelock with the disable bit unset.
    pub fn new(
        claim_pubkey: XOnlyPublicKey,
        refund_pubkey: XOnlyPublicKey,
        payment_hash: [u8; 32],
        csv_blocks: u32,
        blinding_pubkey: BlindingPublicKey,
    ) -> Result<Self, Error> {
        let internal_key = validate_parameters(claim_pubkey, refund_pubkey, csv_blocks)?;
        let liquid_internal_key = LiquidXOnlyPublicKey::from_slice(&internal_key.serialize())
            .map_err(|_| Error::msg("taproot build failed"))?;
        let claim_script = build_claim_script(&claim_pubkey, &payment_hash);
        let refund_script = build_refund_script(&refund_pubkey, csv_blocks);
        let secp = elements::secp256k1_zkp::Secp256k1::new();

        let spend_info = TaprootBuilder::new()
            .add_leaf(1, claim_script.clone())
            .map_err(|_| Error::msg("taproot build failed"))?
            .add_leaf(1, refund_script.clone())
            .map_err(|_| Error::msg("taproot build failed"))?
            .finalize(&secp, liquid_internal_key)
            .map_err(|_| Error::msg("taproot build failed"))?;

        let merkle_root = spend_info
            .merkle_root()
            .ok_or(Error::msg("taproot build failed"))?;
        let leaf_version = LeafVersion::default();
        let claim_leaf_hash = TapLeafHash::from_script(&claim_script, leaf_version);
        let refund_leaf_hash = TapLeafHash::from_script(&refund_script, leaf_version);
        let claim_control_block = spend_info
            .control_block(&(claim_script.clone(), leaf_version))
            .ok_or(Error::msg("taproot build failed"))?;
        let refund_control_block = spend_info
            .control_block(&(refund_script.clone(), leaf_version))
            .ok_or(Error::msg("taproot build failed"))?;

        Ok(Self {
            internal_key,
            output_key: spend_info.output_key(),
            tap_tweak: spend_info.tap_tweak(),
            merkle_root,
            csv_blocks,
            claim_script,
            refund_script,
            claim_leaf_hash,
            refund_leaf_hash,
            claim_control_block,
            refund_control_block,
            blinding_pubkey,
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

    /// Tweaked Elements taproot output key.
    pub fn output_key(&self) -> TweakedPublicKey {
        self.output_key
    }

    /// Elements-domain tap tweak used by cooperative keypath signing.
    pub fn tap_tweak(&self) -> TapTweakHash {
        self.tap_tweak
    }

    /// Elements-domain tap tree merkle root.
    pub fn merkle_root(&self) -> TapNodeHash {
        self.merkle_root
    }

    /// Leaf script for the claim-by-preimage path.
    pub fn claim_script(&self) -> &Script {
        &self.claim_script
    }

    /// Leaf script for the refund-by-CSV path.
    pub fn refund_script(&self) -> &Script {
        &self.refund_script
    }

    /// Elements tapleaf hash for the claim script.
    pub fn claim_leaf_hash(&self) -> TapLeafHash {
        self.claim_leaf_hash
    }

    /// Elements tapleaf hash for the refund script.
    pub fn refund_leaf_hash(&self) -> TapLeafHash {
        self.refund_leaf_hash
    }

    /// Control block for a claim script-path witness.
    pub fn claim_control_block(&self) -> &ControlBlock {
        &self.claim_control_block
    }

    /// Control block for a refund script-path witness.
    pub fn refund_control_block(&self) -> &ControlBlock {
        &self.refund_control_block
    }

    /// Public key used to blind the confidential deposit address.
    pub fn blinding_pubkey(&self) -> BlindingPublicKey {
        self.blinding_pubkey
    }

    /// Liquid P2TR `scriptPubKey` for the opening output.
    pub fn script_pubkey(&self) -> Script {
        Script::new_v1_p2tr_tweaked(self.output_key)
    }

    /// Confidential P2TR address for the opening output.
    pub fn confidential_address(&self, network: SwapNetwork) -> Address {
        Address::p2tr_tweaked(
            self.output_key,
            Some(self.blinding_pubkey),
            network.liquid_address_params(),
        )
    }
}

impl LiquidSwapScript {
    pub fn address(&self, network: SwapNetwork) -> String {
        self.confidential_address(network).to_string()
    }
}

/// `OP_SIZE <32> OP_EQUALVERIFY OP_HASH160 <RIPEMD160(H)> OP_EQUALVERIFY <A> OP_CHECKSIG`
///
/// The size check keeps the leaf spendable only by a preimage Lightning can carry:
/// the payer picks `H`, and a shorter one would take the output while the HTLC
/// stays unsettled.
fn build_claim_script(claim_pubkey: &XOnlyPublicKey, payment_hash: &[u8; 32]) -> Script {
    let ripemd = ripemd160::Hash::hash(payment_hash);
    Builder::new()
        .push_opcode(OP_SIZE)
        .push_int(32)
        .push_opcode(OP_EQUALVERIFY)
        .push_opcode(OP_HASH160)
        .push_slice(ripemd.as_byte_array())
        .push_opcode(OP_EQUALVERIFY)
        .push_slice(&claim_pubkey.serialize())
        .push_opcode(OP_CHECKSIG)
        .into_script()
}

/// `<B> OP_CHECKSIGVERIFY <N> OP_CHECKSEQUENCEVERIFY`
fn build_refund_script(refund_pubkey: &XOnlyPublicKey, csv_blocks: u32) -> Script {
    Builder::new()
        .push_slice(&refund_pubkey.serialize())
        .push_opcode(OP_CHECKSIGVERIFY)
        .push_int(csv_blocks as i64)
        .push_opcode(OP_CSV)
        .into_script()
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        hashes::sha256,
        secp256k1::{SecretKey, ecdh::SharedSecret},
    };
    use elements::{
        opcodes::All as Opcode,
        script::Instruction,
        secp256k1_zkp::{PublicKey, Secp256k1, SecretKey as BlindingSecretKey},
    };

    use super::*;
    use crate::{musig, script::BitcoinSwapScript};

    struct Fixture {
        claim_pk: XOnlyPublicKey,
        refund_pk: XOnlyPublicKey,
        payment_hash: [u8; 32],
        csv_blocks: u32,
        script: LiquidSwapScript,
    }

    fn xonly(seed: [u8; 32]) -> XOnlyPublicKey {
        SecretKey::from_slice(&seed)
            .unwrap()
            .x_only_public_key(crate::secp())
            .0
    }

    fn blinding_pubkey(seed: [u8; 32]) -> BlindingPublicKey {
        let secp = Secp256k1::new();
        let secret = BlindingSecretKey::from_slice(&seed).unwrap();
        PublicKey::from_secret_key(&secp, &secret)
    }

    fn opcodes(script: &Script) -> Vec<Opcode> {
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
        let script = LiquidSwapScript::new(
            claim_pk,
            refund_pk,
            payment_hash,
            csv_blocks,
            blinding_pubkey([3; 32]),
        )
        .unwrap();

        Fixture {
            claim_pk,
            refund_pk,
            payment_hash,
            csv_blocks,
            script,
        }
    }

    #[test]
    fn shared_blinding_key_is_symmetric() {
        let user_key = SecretKey::from_slice(&[3; 32]).unwrap();
        let service_key = SecretKey::from_slice(&[4; 32]).unwrap();
        let user_pubkey = user_key.public_key(crate::secp());
        let service_pubkey = service_key.public_key(crate::secp());

        let (user_blinding_pubkey, user_shared_key) =
            derive_liquid_blinding_key(&user_key, &service_pubkey).unwrap();
        let (service_blinding_pubkey, service_shared_key) =
            derive_liquid_blinding_key(&service_key, &user_pubkey).unwrap();

        assert_eq!(user_shared_key, service_shared_key);
        assert_eq!(user_blinding_pubkey, service_blinding_pubkey);
        assert_eq!(
            user_shared_key.secret_bytes(),
            SharedSecret::new(&service_pubkey, &user_key).secret_bytes()
        );
    }

    #[test]
    fn rejects_duplicate_pubkeys() {
        let f = fixture();
        let err = LiquidSwapScript::new(
            f.claim_pk,
            f.claim_pk,
            f.payment_hash,
            f.csv_blocks,
            blinding_pubkey([3; 32]),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "claim and refund pubkeys must differ");
    }

    #[test]
    fn rejects_out_of_range_csv() {
        let f = fixture();
        for bad in [0u32, 0x10000] {
            let err = LiquidSwapScript::new(
                f.claim_pk,
                f.refund_pk,
                f.payment_hash,
                bad,
                blinding_pubkey([3; 32]),
            )
            .unwrap_err();
            assert_eq!(
                err.to_string(),
                format!("csv_blocks must be in 1..=65535, got {bad}"),
            );
        }
    }

    #[test]
    fn internal_key_matches_musig_aggregate() {
        let f = fixture();
        let expected = musig::aggregate_xonly(f.claim_pk, f.refund_pk).unwrap();
        assert_eq!(f.script.internal_key(), expected);
    }

    #[test]
    fn leaves_match_bitcoin_script_bytes() {
        let f = fixture();
        let bitcoin =
            BitcoinSwapScript::new(f.claim_pk, f.refund_pk, f.payment_hash, f.csv_blocks).unwrap();

        assert_eq!(
            f.script.claim_script().as_bytes(),
            bitcoin.claim_script().as_bytes(),
        );
        assert_eq!(
            f.script.refund_script().as_bytes(),
            bitcoin.refund_script().as_bytes(),
        );
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
        assert_eq!(
            opcodes(f.script.refund_script()),
            vec![OP_CHECKSIGVERIFY, OP_CSV],
        );
    }

    #[test]
    fn control_blocks_commit_to_their_leaves() {
        let f = fixture();
        let secp = Secp256k1::new();

        assert!(f.script.claim_control_block().verify_taproot_commitment(
            &secp,
            &f.script.output_key(),
            f.script.claim_script(),
        ));
        assert!(f.script.refund_control_block().verify_taproot_commitment(
            &secp,
            &f.script.output_key(),
            f.script.refund_script(),
        ));
    }

    #[test]
    fn address_is_confidential_and_matches_script_pubkey() {
        let f = fixture();

        for network in [
            SwapNetwork::Mainnet,
            SwapNetwork::Testnet,
            SwapNetwork::Signet,
            SwapNetwork::Regtest,
        ] {
            let address = f.script.confidential_address(network);
            let expected_params = match network {
                SwapNetwork::Mainnet => &elements::AddressParams::LIQUID,
                SwapNetwork::Testnet | SwapNetwork::Signet => {
                    &elements::AddressParams::LIQUID_TESTNET
                }
                SwapNetwork::Regtest => &elements::AddressParams::ELEMENTS,
            };
            assert!(address.is_blinded());
            assert_eq!(address.params, expected_params);
            assert_eq!(address.blinding_pubkey, Some(f.script.blinding_pubkey()));
            assert_eq!(address.script_pubkey(), f.script.script_pubkey());
            assert_eq!(f.script.address(network), address.to_string());
        }
    }
}
