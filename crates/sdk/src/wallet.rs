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
//! Keys are derived deterministically from the `swap_id` over a [`Keyring`]:
//! `SHA-256(xpriv_secret_bytes || swap_id_bytes)`
//!
//! The identity xpub uses branch 0 (`m/84'/COIN'/0'`), swap keys use branch 1
//! (`m/84'/COIN'/1'`), source and destination blinding keys use branches 2 and 4
//! (`m/84'/COIN'/{2,4}'`, Liquid only), and preimages use branch 3
//! (`m/84'/COIN'/3'`). `COIN` is `0` for the Bitcoin main network, `1` otherwise.
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
    hashes::{Hash, sha256},
    secp256k1::{PublicKey, SecretKey},
};
use thiserror::Error;
use uuid::Uuid;

const BRANCH_XPUB: u32 = 0;
const BRANCH_KEYS: u32 = 1;
const BRANCH_SOURCE_BLINDING: u32 = 2;
const BRANCH_PREIMAGES: u32 = 3;
const BRANCH_DEST_BLINDING: u32 = 4;

/// Wallet trait providing per-swap key derivation.
pub trait SwapWallet {
    /// Error returned by the derivations below. It has to be printable so
    /// callers can report it.
    type Error: std::fmt::Display;

    /// The network this wallet derives keys for.
    fn network(&self) -> SwapNetwork;

    /// Stable pubkey identifying this wallet.
    fn identity_pubkey(&self) -> Result<PublicKey, Self::Error>;

    /// Derives the compressed public key for the given `swap_id`.
    fn pubkey(&self, swap_id: &Uuid) -> Result<PublicKey, Self::Error>;

    /// Derives the per-swap signing private key for the given `swap_id`.
    fn privkey(&self, swap_id: &Uuid) -> Result<SecretKey, Self::Error>;

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
        let seed = mnemonic.to_seed("");
        let keyring = Keyring::from_seed(
            &seed,
            network,
            &[
                BRANCH_XPUB,
                BRANCH_KEYS,
                BRANCH_SOURCE_BLINDING,
                BRANCH_PREIMAGES,
                BRANCH_DEST_BLINDING,
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

    fn pubkey(&self, swap_id: &Uuid) -> Result<PublicKey, WalletError> {
        Ok(self.privkey(swap_id)?.public_key(secp()))
    }

    fn privkey(&self, swap_id: &Uuid) -> Result<SecretKey, WalletError> {
        Ok(self.keyring.swap_key(BRANCH_KEYS, swap_id)?)
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
