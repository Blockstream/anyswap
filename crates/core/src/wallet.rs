//! Deterministic key derivation shared by both wallets.
//!
//! [`Keyring`] is a small BIP32 building block: from a seed it derives a fixed
//! set of hardened branches up front, then serves per-swap keys off them by
//! `swap_id`. Both the SDK's user wallet and the plugin's node wallet are thin
//! role-naming wrappers over one.
//!
//! Derived private keys are exposed in the clear by design: a cooperative
//! keypath spend reveals one side's per-swap key so the counterparty can
//! compose the Schnorr signature locally. Hardware-only wallets that can't
//! expose per-swap signing keys aren't supported.

use std::collections::BTreeMap;

use bitcoin::{
    bip32::{ChildNumber, DerivationPath, Xpriv},
    hashes::{Hash, HashEngine, sha256},
    secp256k1::{PublicKey, SecretKey},
};
use thiserror::Error;
use uuid::Uuid;

use crate::{secp, types::SwapNetwork};

const PURPOSE: u32 = 84;

#[derive(Debug, Error)]
pub enum KeyringError {
    #[error("BIP32 derivation: {0}")]
    Bip32(#[from] bitcoin::bip32::Error),

    #[error("invalid secp256k1 scalar: {0}")]
    Scalar(#[from] bitcoin::secp256k1::Error),

    #[error("branch {0} was not declared at construction")]
    UnknownBranch(u32),
}

/// A deterministic keyring: a set of hardened branches derived once from a seed,
/// plus per-swap keys served off them.
///
/// The caller declares which branch indices it uses at construction; the master
/// key is derived, used, and dropped, so a per-swap key later costs a single
/// hash. Immutable once built, so a single instance is meant to be shared across
/// the whole process (an `Arc` is enough; no locking).
#[derive(Clone)]
pub struct Keyring {
    branches: BTreeMap<u32, Xpriv>,
}

impl Keyring {
    /// Builds a keyring from a domain-separated seed, deriving one hardened
    /// branch per index in `branches`.
    pub fn from_seed(
        seed: &[u8],
        network: SwapNetwork,
        branches: &[u32],
    ) -> Result<Self, KeyringError> {
        let master = Xpriv::new_master(network.bitcoin_network(), seed)?;

        let branches = branches
            .iter()
            .map(|&branch| Ok((branch, derive_branch(&master, network, branch)?)))
            .collect::<Result<_, KeyringError>>()?;

        Ok(Self { branches })
    }

    /// Derives the per-swap private key on `branch` for `swap_id`.
    pub fn swap_key(&self, branch: u32, swap_id: &Uuid) -> Result<SecretKey, KeyringError> {
        Ok(derive_swap_key(self.branch(branch)?, swap_id)?)
    }

    /// The branch's own private key (the hardened branch xpriv's key).
    pub fn branch_privkey(&self, branch: u32) -> Result<SecretKey, KeyringError> {
        Ok(self.branch(branch)?.private_key)
    }

    /// The compressed public key of [`branch_privkey`](Self::branch_privkey).
    pub fn branch_pubkey(&self, branch: u32) -> Result<PublicKey, KeyringError> {
        Ok(self.branch_privkey(branch)?.public_key(secp()))
    }

    /// Looks up a branch registered at construction. Branch indices are fixed
    /// constants, so a miss is a programmer error rather than a runtime
    /// condition, but it is reported instead of panicking: this is a library.
    fn branch(&self, branch: u32) -> Result<&Xpriv, KeyringError> {
        self.branches
            .get(&branch)
            .ok_or(KeyringError::UnknownBranch(branch))
    }
}

/// Derives the hardened branch `m/84'/<coin>'/<branch>'` from a wallet master
/// key. `<coin>` is `0` on mainnet and `1` on every test network, so test
/// networks share their keys with each other.
fn derive_branch(
    master: &Xpriv,
    network: SwapNetwork,
    branch: u32,
) -> Result<Xpriv, bitcoin::bip32::Error> {
    let coin = match network {
        SwapNetwork::Mainnet => 0,
        _ => 1,
    };
    let path = DerivationPath::from(vec![
        ChildNumber::from_hardened_idx(PURPOSE).expect("fixed purpose"),
        ChildNumber::from_hardened_idx(coin).expect("fixed coin"),
        ChildNumber::from_hardened_idx(branch).expect("fixed branch"),
    ]);
    master.derive_priv(secp(), &path)
}

/// Derives a per-swap secret as `SHA-256(xpriv_secret_bytes || swap_id_bytes)`.
///
/// The branch `xpriv` domain-separates the key; `swap_id` selects it within that
/// branch. The hash is one-way, so revealing a derived key - which a cooperative
/// keypath spend does by design - exposes neither the branch key nor the keys of
/// any other swap.
fn derive_swap_key(xpriv: &Xpriv, swap_id: &Uuid) -> Result<SecretKey, bitcoin::secp256k1::Error> {
    let mut engine = sha256::Hash::engine();
    engine.input(&xpriv.private_key.secret_bytes());
    engine.input(swap_id.as_bytes());
    let hash = sha256::Hash::from_engine(engine);
    SecretKey::from_slice(hash.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [0x42; 32];
    const SWAP_ID: &str = "00112233-4455-6677-8899-aabbccddeeff";

    // BIP39 seed of the standard test mnemonic with an empty passphrase, from
    // which the SDK user wallet derives.
    const BIP39_TEST_SEED: &str = "5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc19a5ac40b389cd370d086206dec8aa6c43daea6690f20ad3d8d48b2d2ce9e38e4";

    fn keyring(network: SwapNetwork) -> Keyring {
        Keyring::from_seed(&SEED, network, &[0, 1]).unwrap()
    }

    fn swap_id() -> Uuid {
        Uuid::parse_str(SWAP_ID).unwrap()
    }

    // A change here is a breaking protocol change, never a refactor.
    #[test]
    fn keyring_derivation_vectors() {
        let swap_id = swap_id();
        let cases = [
            (
                SwapNetwork::Mainnet,
                "0f2e2ea115f409ee6e6483e3bd2670fcd75d7d40403f427e267889ee1789493e",
                "8b376168533a879dc6e2e0fb30a6e6d05e646732df386971504e40b32ace99f8",
            ),
            (
                SwapNetwork::Regtest,
                "28734f9f0d768466462ec6c6061cff4246bb18e07048861a153929f534b16bb4",
                "1f324e1a744da50a1654784536741afa11ba03020175862082ee9e5a3be716a2",
            ),
        ];

        for (network, branch0, branch1) in cases {
            let keyring = keyring(network);
            assert_eq!(
                hex::encode(keyring.swap_key(0, &swap_id).unwrap().secret_bytes()),
                branch0,
                "branch 0 on {network:?}"
            );
            assert_eq!(
                hex::encode(keyring.swap_key(1, &swap_id).unwrap().secret_bytes()),
                branch1,
                "branch 1 on {network:?}"
            );
        }
    }

    #[test]
    fn keyring_mainnet_differs_from_test_networks() {
        let swap_id = swap_id();
        assert_ne!(
            keyring(SwapNetwork::Mainnet).swap_key(0, &swap_id).unwrap(),
            keyring(SwapNetwork::Regtest).swap_key(0, &swap_id).unwrap()
        );
    }

    #[test]
    fn keyring_test_networks_share_derivation() {
        let swap_id = swap_id();
        let testnet = keyring(SwapNetwork::Testnet);
        let signet = keyring(SwapNetwork::Signet);
        let regtest = keyring(SwapNetwork::Regtest);

        assert_eq!(
            testnet.swap_key(0, &swap_id).unwrap(),
            signet.swap_key(0, &swap_id).unwrap()
        );
        assert_eq!(
            testnet.swap_key(0, &swap_id).unwrap(),
            regtest.swap_key(0, &swap_id).unwrap()
        );
    }

    #[test]
    fn keyring_derivation_is_deterministic_and_branch_separated() {
        let keyring = keyring(SwapNetwork::Regtest);
        let swap_id = Uuid::from_u128(0x1234567890abcdef1234567890abcdef);

        assert_eq!(
            keyring.swap_key(0, &swap_id).unwrap(),
            keyring.swap_key(0, &swap_id).unwrap()
        );
        assert_ne!(
            keyring.swap_key(0, &swap_id).unwrap(),
            keyring.swap_key(1, &swap_id).unwrap()
        );
        assert_ne!(
            keyring.swap_key(0, &swap_id).unwrap(),
            keyring
                .swap_key(0, &Uuid::from_u128(swap_id.as_u128() + 1))
                .unwrap()
        );
    }

    #[test]
    fn branch_pubkey_matches_branch_privkey() {
        let keyring = keyring(SwapNetwork::Regtest);
        assert_eq!(
            keyring.branch_pubkey(0).unwrap(),
            keyring.branch_privkey(0).unwrap().public_key(secp())
        );
    }

    #[test]
    fn unregistered_branch_is_reported() {
        let keyring = keyring(SwapNetwork::Regtest);
        assert!(matches!(
            keyring.swap_key(9, &swap_id()),
            Err(KeyringError::UnknownBranch(9))
        ));
        assert!(matches!(
            keyring.branch_privkey(9),
            Err(KeyringError::UnknownBranch(9))
        ));
        assert!(matches!(
            keyring.branch_pubkey(9),
            Err(KeyringError::UnknownBranch(9))
        ));
    }

    // A change here breaks every user wallet already in use.
    #[test]
    fn keyring_user_wallet_derivation_vectors() {
        let seed = hex::decode(BIP39_TEST_SEED).unwrap();
        let swap_id = swap_id();

        let mainnet = Keyring::from_seed(&seed, SwapNetwork::Mainnet, &[0, 1, 2, 3]).unwrap();
        let signing = mainnet.swap_key(1, &swap_id).unwrap();
        assert_eq!(
            hex::encode(mainnet.branch_privkey(0).unwrap().secret_bytes()),
            "e14f274d16ca0d91031b98b162618061d03930fa381af6d4caf44b01819ab6d4"
        );
        assert_eq!(
            hex::encode(mainnet.branch_pubkey(0).unwrap().serialize()),
            "02707a62fdacc26ea9b63b1c197906f56ee0180d0bcf1966e1a2da34f5f3a09a9b"
        );
        assert_eq!(
            hex::encode(signing.secret_bytes()),
            "64aff91bfd58a39f00963638da6c40ddb82052abc62b894845ec5456f3588bf4"
        );
        assert_eq!(
            hex::encode(signing.public_key(secp()).serialize()),
            "02dfde19a23be257d8cf6c7ca55c8fee3221e55f2f34453fe8350b02938ba82838"
        );
        assert_eq!(
            hex::encode(mainnet.swap_key(2, &swap_id).unwrap().secret_bytes()),
            "c4b95f771db2ab111c22c41f462c87c59cb3bb7363c8ce9e668c02b68840ca37"
        );
        assert_eq!(
            hex::encode(mainnet.swap_key(3, &swap_id).unwrap().secret_bytes()),
            "75e7aaa283daca0796ec1b0b18de2bc66ba4b96f6772a7dd2dd672ead8837cdf"
        );

        let regtest = Keyring::from_seed(&seed, SwapNetwork::Regtest, &[0, 1, 2, 3]).unwrap();
        assert_eq!(
            hex::encode(regtest.branch_privkey(0).unwrap().secret_bytes()),
            "7262788152f6450e0f0b336847e5ed3ea4319e10b793c3a7488a474aa4fbeaae"
        );
        assert_eq!(
            hex::encode(regtest.swap_key(1, &swap_id).unwrap().secret_bytes()),
            "396a880b5f9084de3cb2fc641e10739752fd90e196c58f9330069174f7c44a17"
        );
        assert_eq!(
            hex::encode(regtest.swap_key(2, &swap_id).unwrap().secret_bytes()),
            "f82050d0a8fc6c72cb88191dd2041ec71719d4af694bb01637f1390a3f511255"
        );
        assert_eq!(
            hex::encode(regtest.swap_key(3, &swap_id).unwrap().secret_bytes()),
            "bb5ede6f2bbc0173e9af5b64e4f52d60fbf19c8c119aceb1e89f6a11f460a222"
        );
    }
}
