//! BIP327 key aggregation and local MuSig2 signing after counterparty privkey reveal.
//!
//! The keypath spend is non-interactive: once the counterparty reveals their half,
//! the claimer holds both private keys and runs the protocol locally end-to-end.

use bitcoin::{
    TapTweakHash,
    hashes::Hash,
    secp256k1::{SecretKey, XOnlyPublicKey},
    taproot::TapNodeHash,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MusigError {
    #[error("invalid pubkey: {0}")]
    InvalidPubkey(String),

    #[error("invalid privkey: {0}")]
    InvalidPrivkey(String),

    #[error("invalid tap tweak: {0}")]
    InvalidTapTweak(String),

    #[error("aggregated signature failed verification")]
    InvalidSignature,
}

/// Aggregates two x-only pubkeys via BIP327 KeyAgg. Order-independent.
pub fn aggregate_xonly(a: XOnlyPublicKey, b: XOnlyPublicKey) -> Result<XOnlyPublicKey, MusigError> {
    let cache = build_cache(lift(a)?, lift(b)?);
    from_secp256k1_xonly(cache.agg_pk())
}

/// Schnorr signature under the BIP327 aggregate key, tweaked for the taproot
/// output. Requires both privkeys; the counterparty's is revealed during coop
/// close, after which the keypath spend is non-interactive.
///
/// `merkle_root`: `Some` for script-path availability, `None` for keypath-only.
pub fn sign_keypath(
    sk_a: SecretKey,
    sk_b: SecretKey,
    merkle_root: Option<TapNodeHash>,
    msg: &[u8; 32],
) -> Result<[u8; 64], MusigError> {
    let internal_key = aggregate_xonly(
        sk_a.x_only_public_key(crate::secp()).0,
        sk_b.x_only_public_key(crate::secp()).0,
    )?;
    let tweak = TapTweakHash::from_key_and_tweak(internal_key, merkle_root);
    sign_keypath_with_tweak(sk_a, sk_b, tweak.to_byte_array(), msg)
}

/// Schnorr signature under a BIP327 aggregate key with a caller-provided
/// x-only tweak. Liquid uses the same MuSig2 protocol as Bitcoin but computes
/// its tap tweak in the Elements tagged-hash domain.
pub fn sign_keypath_with_tweak(
    sk_a: SecretKey,
    sk_b: SecretKey,
    tweak: [u8; 32],
    msg: &[u8; 32],
) -> Result<[u8; 64], MusigError> {
    let kp_a = even_y_keypair(sk_a)?;
    let kp_b = even_y_keypair(sk_b)?;

    let mut cache = build_cache(kp_a.public_key(), kp_b.public_key());
    apply_xonly_tweak(&mut cache, tweak)?;
    let output_key = cache.agg_pk();

    let mut rng = secp256k1::rand::rng();
    let mut nonce = |kp: &secp256k1::Keypair| {
        cache.nonce_gen_with_uniform_randomness(
            secp256k1::musig::SessionSecretRand::from_rng(&mut rng),
            kp.public_key(),
            msg,
            secp256k1::rand::random(),
        )
    };
    let (sn_a, pn_a) = nonce(&kp_a);
    let (sn_b, pn_b) = nonce(&kp_b);

    let agg_nonce = secp256k1::musig::AggregatedNonce::new(&[&pn_a, &pn_b]);
    let session = secp256k1::musig::Session::new(&cache, agg_nonce, msg);
    let ps_a = session.partial_sign(sn_a, &kp_a, &cache);
    let ps_b = session.partial_sign(sn_b, &kp_b, &cache);

    let sig = session
        .partial_sig_agg(&[&ps_a, &ps_b])
        .verify(&output_key, msg)
        .map_err(|_| MusigError::InvalidSignature)?;
    Ok(sig.to_byte_array())
}

fn build_cache(
    p_a: secp256k1::PublicKey,
    p_b: secp256k1::PublicKey,
) -> secp256k1::musig::KeyAggCache {
    let mut pks = [&p_a, &p_b];
    secp256k1::sort_pubkeys(&mut pks);
    secp256k1::musig::KeyAggCache::new(&pks)
}

fn lift(x: XOnlyPublicKey) -> Result<secp256k1::PublicKey, MusigError> {
    let secp256k1_x = secp256k1::XOnlyPublicKey::from_byte_array(x.serialize())
        .map_err(|e| MusigError::InvalidPubkey(e.to_string()))?;
    Ok(secp256k1::PublicKey::from_x_only_public_key(
        secp256k1_x,
        secp256k1::Parity::Even,
    ))
}

fn from_secp256k1_xonly(x: secp256k1::XOnlyPublicKey) -> Result<XOnlyPublicKey, MusigError> {
    XOnlyPublicKey::from_slice(&x.to_byte_array())
        .map_err(|e| MusigError::InvalidPubkey(e.to_string()))
}

fn even_y_keypair(sk: SecretKey) -> Result<secp256k1::Keypair, MusigError> {
    let kp = secp256k1::SecretKey::from_secret_bytes(sk.secret_bytes())
        .map_err(|e| MusigError::InvalidPrivkey(e.to_string()))?
        .keypair();
    Ok(match kp.x_only_public_key().1 {
        secp256k1::Parity::Even => kp,
        secp256k1::Parity::Odd => kp.secret_key().negate().keypair(),
    })
}

fn apply_xonly_tweak(
    cache: &mut secp256k1::musig::KeyAggCache,
    tweak: [u8; 32],
) -> Result<(), MusigError> {
    let tweak = secp256k1::Scalar::from_be_bytes(tweak)
        .map_err(|e| MusigError::InvalidTapTweak(e.to_string()))?;
    cache
        .pubkey_xonly_tweak_add(&tweak)
        .map_err(|e| MusigError::InvalidTapTweak(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        key::TapTweak,
        secp256k1::{Message, Secp256k1, schnorr},
    };

    use super::*;

    fn keypair(sk_bytes: [u8; 32]) -> (SecretKey, XOnlyPublicKey) {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&sk_bytes).unwrap();
        let pk = sk.public_key(&secp).x_only_public_key().0;
        (sk, pk)
    }

    #[test]
    fn order_independent() {
        let (_, pk_a) = keypair([1; 32]);
        let (_, pk_b) = keypair([2; 32]);
        assert_eq!(
            aggregate_xonly(pk_a, pk_b).unwrap().serialize(),
            aggregate_xonly(pk_b, pk_a).unwrap().serialize(),
        );
    }

    fn verify(
        sig_bytes: [u8; 64],
        internal: XOnlyPublicKey,
        merkle_root: Option<TapNodeHash>,
        msg: [u8; 32],
    ) {
        let secp = Secp256k1::new();
        let (output_key, _parity) = internal.tap_tweak(&secp, merkle_root);
        let sig = schnorr::Signature::from_slice(&sig_bytes).unwrap();
        let msg_obj = Message::from_digest(msg);
        secp.verify_schnorr(&sig, &msg_obj, &output_key.to_x_only_public_key())
            .expect("schnorr verification");
    }

    #[test]
    fn sign_keypath_with_script_tree() {
        let (sk_a, pk_a) = keypair([1; 32]);
        let (sk_b, pk_b) = keypair([2; 32]);
        let merkle_root = Some(TapNodeHash::from_byte_array([7; 32]));
        let msg = [42u8; 32];

        let sig = sign_keypath(sk_a, sk_b, merkle_root, &msg).unwrap();
        verify(sig, aggregate_xonly(pk_a, pk_b).unwrap(), merkle_root, msg);
    }

    #[test]
    fn sign_keypath_keypath_only() {
        let (sk_a, pk_a) = keypair([1; 32]);
        let (sk_b, pk_b) = keypair([2; 32]);
        let msg = [42u8; 32];

        let sig = sign_keypath(sk_a, sk_b, None, &msg).unwrap();
        verify(sig, aggregate_xonly(pk_a, pk_b).unwrap(), None, msg);
    }

    #[test]
    fn sign_keypath_secret_key_order_independent() {
        let (sk_a, pk_a) = keypair([1; 32]);
        let (sk_b, pk_b) = keypair([2; 32]);
        let merkle_root = Some(TapNodeHash::from_byte_array([7; 32]));
        let msg = [42u8; 32];
        let internal = aggregate_xonly(pk_a, pk_b).unwrap();

        let sig_ab = sign_keypath(sk_a, sk_b, merkle_root, &msg).unwrap();
        let sig_ba = sign_keypath(sk_b, sk_a, merkle_root, &msg).unwrap();
        verify(sig_ab, internal, merkle_root, msg);
        verify(sig_ba, internal, merkle_root, msg);
    }
}
