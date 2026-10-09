//! BIP39/BIP32 HD wallet for swap key management.
//!
//! Each swap requires an ephemeral keypair: the user's pubkey goes into the swap
//! script, and the private key is needed later to sign refund transactions or is
//! revealed to the server for a cooperative keypath claim. This wallet manages
//! those keys - it never holds user balances (deposits go directly to the P2TR
//! swap address).
//!
//! ## Key derivation
//!
//! The BIP39 seed is first domain-separated as `HMAC-SHA512(SEED_TAG, seed)`,
//! so no BIP32 wallet on the same mnemonic shares a key with this one. A
//! [`Keyring`] over the result derives the hardened branches
//! `m/84'/COIN'/<branch>'`, and each per-swap key is
//! `SHA-256(xpriv_secret_bytes || swap_id_bytes)` on its branch.
//!
//! Branches 0 and 1 hold the source refund and destination claim keys, 2 and 3
//! the source and destination blinding keys (Liquid only), 4 the preimages and
//! 5 the identity key. `COIN` is `0` for the Bitcoin main network, `1`
//! otherwise. Every role has its own branch because a SwapChain holds both
//! sides under one `swap_id`, and revealing its source refund key must not give
//! away its destination claim key.
//!
//! Only the mnemonic is needed for recovery - the server holds the full swap
//! history (query via [`HttpClient::list_swaps`](crate::client::HttpClient::list_swaps)).

use anyswap_core::{
    secp,
    types::SwapNetwork,
    wallet::{Keyring, KeyringError},
};
use bip39::Mnemonic;
use bitcoin::{
    hashes::{Hash, HashEngine, hmac, sha256, sha512},
    secp256k1::{PublicKey, SecretKey},
};
use thiserror::Error;
use uuid::Uuid;

/// HMAC key that domain-separates the BIP39 seed before any BIP32 derivation.
///
/// Fixed by the protocol: together with the mnemonic it *is* the wallet, and
/// changing it strands every swap made under it. Wallets before `v2` derived
/// from the untagged seed, where these branches were BIP84 account keys.
const SEED_TAG: &str = "anyswap/user-wallet/v2";

const BRANCH_SOURCE_REFUND: u32 = 0;
const BRANCH_DEST_CLAIM: u32 = 1;
const BRANCH_SOURCE_BLINDING: u32 = 2;
const BRANCH_DEST_BLINDING: u32 = 3;
const BRANCH_PREIMAGES: u32 = 4;
const BRANCH_XPUB: u32 = 5;

/// Wallet trait providing per-swap key derivation.
pub trait SwapWallet {
    /// Error returned by the derivations below. It has to be printable so
    /// callers can report it.
    type Error: std::fmt::Display;

    /// The network this wallet derives keys for.
    fn network(&self) -> SwapNetwork;

    /// Stable pubkey identifying this wallet.
    fn identity_pubkey(&self) -> Result<PublicKey, Self::Error>;

    /// Source refund pubkey: the user's key on a source output the user opens
    /// (SwapIn, SwapChain), sent to the server as `source_refund_pubkey`.
    fn source_refund_pubkey(&self, swap_id: &Uuid) -> Result<PublicKey, Self::Error>;

    /// Source refund private key: signs the refund, and is revealed to the
    /// server for a cooperative claim of the source.
    fn source_refund_privkey(&self, swap_id: &Uuid) -> Result<SecretKey, Self::Error>;

    /// Destination claim pubkey: the user's key on a destination output the
    /// server opens (SwapOut, SwapChain), sent to the server as
    /// `dest_claim_pubkey`.
    fn dest_claim_pubkey(&self, swap_id: &Uuid) -> Result<PublicKey, Self::Error>;

    /// Destination claim private key: signs the claim of the destination.
    fn dest_claim_privkey(&self, swap_id: &Uuid) -> Result<SecretKey, Self::Error>;

    /// Derives the per-swap preimage for the given `swap_id`.
    fn preimage(&self, swap_id: &Uuid) -> Result<[u8; 32], Self::Error>;

    /// Source-side blinding pubkey `U = u·G` (Liquid swap-in): sent to the
    /// server as `source_blinding_pubkey` so it can derive the ECDH address
    /// blinding key.
    fn source_blinding_pubkey(&self, swap_id: &Uuid) -> Result<PublicKey, Self::Error>;

    /// Source-side blinding private key (Liquid swap-in): unblinds the
    /// confidential source deposit output.
    fn source_blinding_privkey(&self, swap_id: &Uuid) -> Result<SecretKey, Self::Error>;

    /// Destination-side blinding pubkey (Liquid swap-out): sent to the server as
    /// `dest_blinding_pubkey`.
    fn dest_blinding_pubkey(&self, swap_id: &Uuid) -> Result<PublicKey, Self::Error>;

    /// Destination-side blinding private key (Liquid swap-out): unblinds the
    /// confidential opening output the user claims.
    fn dest_blinding_privkey(&self, swap_id: &Uuid) -> Result<SecretKey, Self::Error>;
}

/// Extension on top of [`SwapWallet`] with fixed implementations.
/// Added to every `SwapWallet` automatically; wallet implementors can't
/// change them.
pub trait SwapWalletExt: SwapWallet {
    /// Stable pseudonymous identifier sent to the server for routing.
    /// Hex-encoded SHA-256 of the compressed identity pubkey.
    fn user_id(&self) -> Result<String, Self::Error> {
        Ok(sha256::Hash::hash(&self.identity_pubkey()?.serialize()).to_string())
    }

    /// Per-swap payment hash: SHA-256 of the preimage.
    fn payment_hash(&self, swap_id: &Uuid) -> Result<[u8; 32], Self::Error> {
        let preimage = self.preimage(swap_id)?;
        Ok(*sha256::Hash::hash(&preimage).as_byte_array())
    }
}

impl<W: SwapWallet + ?Sized> SwapWalletExt for W {}

#[derive(Debug, Error)]
pub enum WalletError {
    #[error("mnemonic: {0}")]
    Mnemonic(#[from] bip39::Error),

    #[error("keyring: {0}")]
    Keyring(#[from] KeyringError),
}

/// BIP39/BIP32 HD wallet for swap key management. See the [module docs](self) for details.
#[derive(Clone)]
pub struct Wallet {
    keyring: Keyring,
    network: SwapNetwork,
}

impl Wallet {
    /// Generates a new wallet with a random 24-word mnemonic.
    /// Returns both the wallet and the mnemonic so the caller can display/store it.
    pub fn generate(network: SwapNetwork) -> Result<(Self, Mnemonic), WalletError> {
        let mnemonic = Self::generate_mnemonic(24)?;
        let wallet = Self::from_mnemonic(&mnemonic, network)?;
        Ok((wallet, mnemonic))
    }

    /// Generates a random mnemonic; `word_count` must be 12, 15, 18, 21, or 24.
    pub fn generate_mnemonic(word_count: u8) -> Result<Mnemonic, WalletError> {
        Ok(Mnemonic::generate(word_count as usize)?)
    }

    /// Restores a wallet from an existing mnemonic.
    pub fn from_mnemonic(mnemonic: &Mnemonic, network: SwapNetwork) -> Result<Self, WalletError> {
        let mut engine = hmac::HmacEngine::<sha512::Hash>::new(SEED_TAG.as_bytes());
        engine.input(&mnemonic.to_seed(""));
        let seed = hmac::Hmac::<sha512::Hash>::from_engine(engine);
        let keyring = Keyring::from_seed(
            seed.as_byte_array(),
            network,
            &[
                BRANCH_SOURCE_REFUND,
                BRANCH_DEST_CLAIM,
                BRANCH_SOURCE_BLINDING,
                BRANCH_DEST_BLINDING,
                BRANCH_PREIMAGES,
                BRANCH_XPUB,
            ],
        )?;

        Ok(Self { keyring, network })
    }

    /// Returns the stable identity private key used to authenticate API requests.
    pub fn identity_privkey(&self) -> Result<SecretKey, WalletError> {
        Ok(self.keyring.branch_privkey(BRANCH_XPUB)?)
    }
}

impl SwapWallet for Wallet {
    type Error = WalletError;

    fn network(&self) -> SwapNetwork {
        self.network
    }

    fn identity_pubkey(&self) -> Result<PublicKey, WalletError> {
        Ok(self.keyring.branch_pubkey(BRANCH_XPUB)?)
    }

    fn source_refund_pubkey(&self, swap_id: &Uuid) -> Result<PublicKey, WalletError> {
        Ok(self.source_refund_privkey(swap_id)?.public_key(secp()))
    }

    fn source_refund_privkey(&self, swap_id: &Uuid) -> Result<SecretKey, WalletError> {
        Ok(self.keyring.swap_key(BRANCH_SOURCE_REFUND, swap_id)?)
    }

    fn dest_claim_pubkey(&self, swap_id: &Uuid) -> Result<PublicKey, WalletError> {
        Ok(self.dest_claim_privkey(swap_id)?.public_key(secp()))
    }

    fn dest_claim_privkey(&self, swap_id: &Uuid) -> Result<SecretKey, WalletError> {
        Ok(self.keyring.swap_key(BRANCH_DEST_CLAIM, swap_id)?)
    }

    fn preimage(&self, swap_id: &Uuid) -> Result<[u8; 32], WalletError> {
        Ok(self
            .keyring
            .swap_key(BRANCH_PREIMAGES, swap_id)?
            .secret_bytes())
    }

    fn source_blinding_pubkey(&self, swap_id: &Uuid) -> Result<PublicKey, WalletError> {
        Ok(self.source_blinding_privkey(swap_id)?.public_key(secp()))
    }

    fn source_blinding_privkey(&self, swap_id: &Uuid) -> Result<SecretKey, WalletError> {
        Ok(self.keyring.swap_key(BRANCH_SOURCE_BLINDING, swap_id)?)
    }

    fn dest_blinding_pubkey(&self, swap_id: &Uuid) -> Result<PublicKey, WalletError> {
        Ok(self.dest_blinding_privkey(swap_id)?.public_key(secp()))
    }

    fn dest_blinding_privkey(&self, swap_id: &Uuid) -> Result<SecretKey, WalletError> {
        Ok(self.keyring.swap_key(BRANCH_DEST_BLINDING, swap_id)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    const SWAP_ID: &str = "00112233-4455-6677-8899-aabbccddeeff";

    #[test]
    fn wallet_derivation_vectors() {
        let mnemonic = Mnemonic::parse(MNEMONIC).unwrap();
        let swap_id = Uuid::parse_str(SWAP_ID).unwrap();
        let cases = [
            (
                SwapNetwork::Mainnet,
                [
                    "17ebb8fbd89e19a204f15db7e7518a64a3faab823fb04f5f9bc8e1bbc1a256e8",
                    "29ae990149ed4887b62d1e54e6cd4965604f7217a10938c8602c3cb6c5ba813d",
                    "15c62260b0e89caa0b99a4bcb5dc8e9601484ea89617fa8c02e512d742caf745",
                    "8531ee614aee023c483953d39f3bc03a0dd687e931023ba9477b12045c442464",
                    "cc0244bdae5324a247cc8fb6be8a2f07170a5d4d01dd217896cbd7d322264a92",
                    "0dd90872f6f2a43e856a1ce3bdcc458cdee77754354fc2b1d887a47364d1711f",
                ],
            ),
            (
                SwapNetwork::Regtest,
                [
                    "8672ce4d21634e6a4baaca46036fd56233e5674fd84c0667a8781bc72e62ad61",
                    "7ab25812b4bd1aa3aea1d8c37a959cae7aafb7c48fc67aa5515e1195eba7890d",
                    "0e69097622e929757e1fd304df690be4cf8657e3d22e87081da5edab107de51f",
                    "3c4006334ae84ccd65500bf3582b385a829c85921bcf100496690fc9d924921a",
                    "20774e9596705e90b6958ce2a54ef4dc91ef7e96f3881bed43fe995a994dea44",
                    "0b623d5f23fba66869ed5bd40001f49c7c2fdb9a71d4a39b6a838ab59ee7a164",
                ],
            ),
        ];

        for (network, expected) in cases {
            let wallet = Wallet::from_mnemonic(&mnemonic, network).unwrap();
            let secret = |key: Result<SecretKey, _>| hex::encode(key.unwrap().secret_bytes());
            let derived = [
                wallet.user_id().unwrap(),
                secret(wallet.source_refund_privkey(&swap_id)),
                secret(wallet.dest_claim_privkey(&swap_id)),
                secret(wallet.source_blinding_privkey(&swap_id)),
                secret(wallet.dest_blinding_privkey(&swap_id)),
                hex::encode(wallet.preimage(&swap_id).unwrap()),
            ];
            assert_eq!(derived, expected, "{network:?}");
        }
    }
}
