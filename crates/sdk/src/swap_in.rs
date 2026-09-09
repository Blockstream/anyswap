//! Typed client-side view of a swap-in.
//!
//! [`SwapIn`] is the parsed form of the wire [`SwapState`]: hex fields become
//! keys and outpoints, and per-status fields move into [`SwapInStatus`] variants
//! so the consumer sees exactly what is available in each state. Created via
//! `TryFrom<SwapState>` on the polling response.

pub use anyswap_core::types::{ErrorType, SwapInStatus as Status};
use anyswap_core::{
    api::SwapState,
    invoice::{self, InvoiceError},
    transaction::TxError,
    types::{Chain, Outpoint, SwapType},
    utils::sha256_payment_hash,
};
use bitcoin::secp256k1::{PublicKey, SecretKey};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    codec::{self, CodecError},
    context::SwapContext,
    swap_script::{ScriptError, Side, SwapScript, Terms, Tx},
    utxo::{SwapUtxo, UtxoError},
    wallet::SwapWallet,
};

#[derive(Debug, Error)]
pub enum SwapInError {
    #[error("expected swap type SwapIn, got {0}")]
    UnexpectedSwapType(SwapType),

    #[error("unknown status: {0}")]
    UnknownStatus(String),

    #[error("unknown error type: {0}")]
    UnknownErrorType(String),

    #[error("missing field: {0}")]
    MissingField(&'static str),

    #[error("a keypath refund needs the claim key the server reveals when it cancels")]
    KeypathRefundUnavailable,

    #[error("this client is not set up for Liquid; add the Liquid parameters to the context")]
    LiquidNotConfigured,

    #[error("the swap trades asset {found}, but this client is set up for {expected}")]
    UnexpectedAsset { expected: String, found: String },

    #[error("wallet: {0}")]
    Wallet(String),

    #[error("invalid swap script: {0}")]
    Script(String),

    #[error("the output does not belong to this swap")]
    UnknownOutput,

    #[error("the server has not reported a preimage in this status")]
    NoPreimage,

    #[error(transparent)]
    Invoice(#[from] InvoiceError),

    #[error("the preimage hashes to {found}, but the claim invoice's payment hash is {expected}")]
    UnexpectedPaymentHash { expected: String, found: String },

    #[error(transparent)]
    Utxo(#[from] UtxoError),

    #[error(transparent)]
    Tx(#[from] TxError),

    #[error(transparent)]
    Codec(#[from] CodecError),
}

impl From<ScriptError> for SwapInError {
    fn from(e: ScriptError) -> Self {
        match e {
            ScriptError::LiquidNotConfigured => Self::LiquidNotConfigured,
            ScriptError::UnexpectedAsset { expected, found } => {
                Self::UnexpectedAsset { expected, found }
            }
            ScriptError::MissingField(field) => Self::MissingField(field),
            ScriptError::Wallet(e) => Self::Wallet(e),
            ScriptError::Script(e) => Self::Script(e),
        }
    }
}

/// A swap-in: the user locks on-chain funds into the swap script and the server
/// pays the user's `claim_invoice` once the opening transaction confirms. The
/// user holds the refund key, the server holds the claim key.
#[derive(Debug, Clone)]
pub struct SwapIn {
    pub swap_id: Uuid,
    pub protocol_version: String,
    pub chain: Chain,
    pub asset: String,
    pub receive_amount: u64,
    pub send_amount: u64,
    pub base_fee: u64,
    pub service_fee: u64,
    pub swap_fee_limit: u64,
    pub payment_hash: [u8; 32],
    pub csv: u32,
    /// User's pubkey in the swap script (refund side).
    pub refund_pubkey: PublicKey,
    /// Server's pubkey in the swap script (claim side).
    pub claim_pubkey: PublicKey,
    /// User's ECDH blinding pubkey for the Liquid opening output.
    pub user_blinding_pubkey: Option<PublicKey>,
    /// Server's ECDH blinding pubkey for the Liquid opening output.
    pub service_blinding_pubkey: Option<PublicKey>,
    pub claim_invoice: String,
    pub refund_address: Option<String>,
    pub created_at: u64,
    pub transitioned_at: u64,
    pub status: SwapInStatus,
}

/// Server-reported swap-in lifecycle state with the fields available in it.
///
/// Normal flow: [`AwaitOpeningTxConfirmation`](Self::AwaitOpeningTxConfirmation) ->
/// [`PayInvoice`](Self::PayInvoice) ->
/// [`AwaitClaimKeyRevealMessage`](Self::AwaitClaimKeyRevealMessage) ->
/// [`AwaitClaimTxConfirmation`](Self::AwaitClaimTxConfirmation) ->
/// [`Claimed`](Self::Claimed).
///
/// Cancellation at any point before a successful claim results in
/// [`Canceled`](Self::Canceled), where the server reveals its claim privkey so
/// the user can refund a deposit without waiting out the CSV.
#[derive(Debug, Clone, strum::Display, strum::IntoStaticStr)]
pub enum SwapInStatus {
    /// Server is watching the swap address for the opening tx and its
    /// confirmations. No server-provided outpoint yet: the client tracks its own
    /// deposit by scanning the swap address until the swap advances.
    AwaitOpeningTxConfirmation,
    /// Opening tx confirmed; server is paying the claim invoice.
    PayInvoice { outpoint: Outpoint },
    /// Invoice paid. `preimage` is the server's evidence of that; check it with
    /// [`SwapIn::verify_payment`] before revealing the refund key. Server awaits
    /// that key (sent via
    /// [`HttpClient::reveal_claim`](crate::client::HttpClient::reveal_claim))
    /// for a cooperative keypath claim; otherwise it claims via the preimage
    /// script path.
    AwaitClaimKeyRevealMessage {
        outpoint: Outpoint,
        preimage: [u8; 32],
    },
    /// Claim tx broadcast; awaiting on-chain confirmation.
    AwaitClaimTxConfirmation {
        outpoint: Outpoint,
        preimage: [u8; 32],
    },
    /// Terminal: claim tx confirmed. Swap finished successfully.
    Claimed {
        outpoint: Outpoint,
        preimage: [u8; 32],
    },
    /// Terminal: canceled before a successful claim. `error_code` is the
    /// machine-readable reason. The server reveals its claim privkey; the user
    /// recovers any deposit by re-deriving the swap address, finding the UTXOs on
    /// chain, and refunding them (keypath, or CSV).
    Canceled {
        error_code: ErrorType,
        claim_privkey: SecretKey,
        cancel_message: String,
    },
}

impl SwapInStatus {
    /// Opening outpoint, once the server has recorded the confirmed deposit.
    /// `Canceled` never carries it; the user re-derives the address to find their
    /// deposit on chain.
    pub fn outpoint(&self) -> Option<&Outpoint> {
        match self {
            Self::AwaitOpeningTxConfirmation | Self::Canceled { .. } => None,
            Self::PayInvoice { outpoint }
            | Self::AwaitClaimKeyRevealMessage { outpoint, .. }
            | Self::AwaitClaimTxConfirmation { outpoint, .. }
            | Self::Claimed { outpoint, .. } => Some(outpoint),
        }
    }

    /// Payment preimage, present from the moment the server paid the claim
    /// invoice.
    pub fn preimage(&self) -> Option<&[u8; 32]> {
        match self {
            Self::AwaitClaimKeyRevealMessage { preimage, .. }
            | Self::AwaitClaimTxConfirmation { preimage, .. }
            | Self::Claimed { preimage, .. } => Some(preimage),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Claimed { .. } | Self::Canceled { .. })
    }
}

/// The user is the refund side of a swap-in, so these build refunds only: a
/// cooperative one once the server reveals its claim key, and the unilateral CSV
/// one otherwise. The per-swap keys come from the context's wallet, so a caller
/// never handles a private key.
impl SwapIn {
    /// Checks the reported preimage against the payment hash in
    /// [`Self::claim_invoice`], which only paying it yields. Call before
    /// [`HttpClient::reveal_claim`](crate::client::HttpClient::reveal_claim).
    /// The invoice is the server's echo of the one sent in `create_swap`;
    /// confirm they match separately.
    pub fn verify_payment(&self) -> Result<(), SwapInError> {
        let expected = invoice::payment_hash(&self.claim_invoice)?;
        let preimage = self.status.preimage().ok_or(SwapInError::NoPreimage)?;
        let found = sha256_payment_hash(preimage);
        if found != expected {
            return Err(SwapInError::UnexpectedPaymentHash {
                expected: hex::encode(expected),
                found: hex::encode(found),
            });
        }
        Ok(())
    }

    /// The deposit address. The user sends `send_amount` here to open the swap.
    pub fn address<W: SwapWallet>(&self, ctx: &SwapContext<W>) -> Result<String, SwapInError> {
        Ok(self.script(ctx)?.address())
    }

    /// Every output of `tx_hex` paying the deposit address: the deposit itself,
    /// and any strays sent to the same address.
    ///
    /// On Liquid an output that does not unblind with this swap's key, or that
    /// holds another asset, belongs to someone else and is skipped.
    pub fn outputs<W: SwapWallet>(
        &self,
        ctx: &SwapContext<W>,
        tx_hex: &str,
    ) -> Result<Vec<SwapUtxo>, SwapInError> {
        let tx = Tx::decode(self.chain, tx_hex)?;
        Ok(self.script(ctx)?.outputs(&tx))
    }

    /// Splits `prevouts` into the ones worth refunding at `fee_rate` and the
    /// ones that would cost more to spend than they return. Run strays through
    /// this before refunding, and show the second set to the user. Needs the
    /// context because Liquid values only exist blinded inside the outputs.
    pub fn partition_economical<W: SwapWallet>(
        &self,
        ctx: &SwapContext<W>,
        utxos: &[SwapUtxo],
        fee_rate: f64,
    ) -> Result<(Vec<SwapUtxo>, Vec<SwapUtxo>), SwapInError> {
        Ok(self.script(ctx)?.economical(utxos, fee_rate)?)
    }

    /// The value of `utxo` in sats. Bitcoin states it on chain; a Liquid value
    /// is blinded, and this unblinds it with the swap's own key.
    pub fn value<W: SwapWallet>(
        &self,
        ctx: &SwapContext<W>,
        utxo: &SwapUtxo,
    ) -> Result<u64, SwapInError> {
        self.script(ctx)?
            .value(utxo)
            .ok_or(SwapInError::UnknownOutput)
    }

    /// Refunds `utxos` cooperatively, spendable right away. Needs the claim key
    /// the server reveals when it cancels the swap. Returns the signed
    /// transaction as hex.
    pub fn refund_by_keypath<W: SwapWallet>(
        &self,
        ctx: &SwapContext<W>,
        utxos: &[SwapUtxo],
        destination: &str,
        fee_rate: f64,
    ) -> Result<String, SwapInError> {
        let SwapInStatus::Canceled { claim_privkey, .. } = &self.status else {
            return Err(SwapInError::KeypathRefundUnavailable);
        };
        let refund_privkey = self.refund_privkey(ctx)?;
        Ok(self.script(ctx)?.keypath(
            utxos,
            *claim_privkey,
            refund_privkey,
            destination,
            fee_rate,
        )?)
    }

    /// Refunds `utxos` unilaterally, valid once every input has aged past the
    /// swap's CSV timeout. Needs only the user's own key, so it works even when
    /// the server does not answer. Returns the signed transaction as hex.
    pub fn refund_by_csv<W: SwapWallet>(
        &self,
        ctx: &SwapContext<W>,
        utxos: &[SwapUtxo],
        destination: &str,
        fee_rate: f64,
    ) -> Result<String, SwapInError> {
        let refund_privkey = self.refund_privkey(ctx)?;
        Ok(self
            .script(ctx)?
            .refund_by_csv(utxos, refund_privkey, destination, fee_rate)?)
    }

    /// The terms the deposit script is built from.
    pub fn terms(&self) -> Terms {
        Terms {
            swap_id: self.swap_id,
            side: Side::Source,
            chain: self.chain,
            asset: self.asset.clone(),
            csv: self.csv,
            payment_hash: self.payment_hash,
            server_pubkey: self.claim_pubkey,
            server_blinding_pubkey: self.service_blinding_pubkey,
        }
    }

    fn script<W: SwapWallet>(&self, ctx: &SwapContext<W>) -> Result<SwapScript, SwapInError> {
        Ok(ctx.script(&self.terms())?)
    }

    /// The user's key in the swap script.
    fn refund_privkey<W: SwapWallet>(
        &self,
        ctx: &SwapContext<W>,
    ) -> Result<SecretKey, SwapInError> {
        ctx.wallet()
            .privkey(&self.swap_id)
            .map_err(|error| SwapInError::Wallet(error.to_string()))
    }
}

impl TryFrom<SwapState> for SwapIn {
    type Error = SwapInError;

    fn try_from(s: SwapState) -> Result<Self, SwapInError> {
        if s.swap_type != SwapType::SwapIn {
            return Err(SwapInError::UnexpectedSwapType(s.swap_type));
        }

        let status: Status = s
            .status
            .parse()
            .map_err(|_| SwapInError::UnknownStatus(s.status.clone()))?;

        let status = match status {
            Status::AwaitOpeningTxConfirmation => SwapInStatus::AwaitOpeningTxConfirmation,
            Status::PayInvoice => SwapInStatus::PayInvoice {
                outpoint: source_outpoint(&s)?,
            },
            Status::AwaitClaimKeyRevealMessage => SwapInStatus::AwaitClaimKeyRevealMessage {
                outpoint: source_outpoint(&s)?,
                preimage: preimage(&s)?,
            },
            Status::AwaitClaimTxConfirmation => SwapInStatus::AwaitClaimTxConfirmation {
                outpoint: source_outpoint(&s)?,
                preimage: preimage(&s)?,
            },
            Status::Claimed => SwapInStatus::Claimed {
                outpoint: source_outpoint(&s)?,
                preimage: preimage(&s)?,
            },
            Status::Canceled => {
                let raw = require(s.error_type.clone(), "error_type")?;
                SwapInStatus::Canceled {
                    error_code: raw
                        .parse()
                        .map_err(|_| SwapInError::UnknownErrorType(raw.clone()))?,
                    claim_privkey: codec::parse_privkey(&require(
                        s.source_claim_privkey.clone(),
                        "source_claim_privkey",
                    )?)?,
                    cancel_message: require(s.cancel_message.clone(), "cancel_message")?,
                }
            }
        };
        let user_blinding_pubkey = s
            .source_user_blinding_pubkey
            .as_deref()
            .map(codec::parse_pubkey)
            .transpose()?;
        let service_blinding_pubkey = s
            .source_service_blinding_pubkey
            .as_deref()
            .map(codec::parse_pubkey)
            .transpose()?;

        Ok(SwapIn {
            swap_id: s.swap_id,
            protocol_version: s.protocol_version,
            chain: require(s.source_chain, "source_chain")?,
            asset: require(s.source_asset, "source_asset")?,
            receive_amount: s.receive_amount,
            send_amount: s.send_amount,
            base_fee: s.base_fee,
            service_fee: s.service_fee,
            swap_fee_limit: s.swap_fee_limit,
            payment_hash: codec::parse_hash32(&s.payment_hash)?,
            csv: require(s.source_csv, "source_csv")?,
            refund_pubkey: codec::parse_pubkey(&require(
                s.source_refund_pubkey,
                "source_refund_pubkey",
            )?)?,
            claim_pubkey: codec::parse_pubkey(&require(
                s.source_claim_pubkey,
                "source_claim_pubkey",
            )?)?,
            user_blinding_pubkey,
            service_blinding_pubkey,
            claim_invoice: require(s.claim_invoice, "claim_invoice")?,
            refund_address: s.user_refund_address,
            created_at: s.created_at,
            transitioned_at: s.transitioned_at,
            status,
        })
    }
}

fn require<T>(opt: Option<T>, field: &'static str) -> Result<T, SwapInError> {
    opt.ok_or(SwapInError::MissingField(field))
}

fn source_outpoint(s: &SwapState) -> Result<Outpoint, SwapInError> {
    Ok(codec::parse_outpoint(
        require(s.source_txid.as_deref(), "source_txid")?,
        require(s.source_vout, "source_vout")?,
    )?)
}

fn preimage(s: &SwapState) -> Result<[u8; 32], SwapInError> {
    Ok(codec::parse_hash32(require(
        s.preimage.as_deref(),
        "preimage",
    )?)?)
}

#[cfg(test)]
mod tests {
    use anyswap_core::{
        script::{BitcoinSwapScript, LiquidSwapScript},
        secp,
        types::{ErrorType, NATIVE_ASSET, SwapNetwork},
    };
    use bitcoin::hashes::{Hash as _, sha256};

    use super::*;
    use crate::{
        utxo::fixtures::{confidential_tx, genesis_hash, liquid_asset, liquid_destination},
        wallet::Wallet,
    };

    const SERVICE_SIGNING_KEY: [u8; 32] = [21; 32];
    const SERVICE_BLINDING_KEY: [u8; 32] = [22; 32];

    fn context() -> SwapContext<Wallet> {
        let (wallet, _) = Wallet::generate(SwapNetwork::Regtest).unwrap();
        SwapContext::new(wallet)
            .with_liquid(&genesis_hash(), &liquid_asset())
            .unwrap()
    }

    fn swap_in(ctx: &SwapContext<Wallet>, chain: Chain, status: SwapInStatus) -> SwapIn {
        let swap_id = Uuid::from_u128(11);
        let service_signing = SecretKey::from_slice(&SERVICE_SIGNING_KEY).unwrap();
        let service_blinding = SecretKey::from_slice(&SERVICE_BLINDING_KEY).unwrap();
        let preimage = ctx.wallet().preimage(&swap_id).unwrap();
        SwapIn {
            swap_id,
            protocol_version: "2.0.0".into(),
            chain,
            asset: match chain {
                Chain::Bitcoin => NATIVE_ASSET.to_string(),
                Chain::Liquid => liquid_asset(),
            },
            receive_amount: 100_000,
            send_amount: 101_000,
            base_fee: 1_000,
            service_fee: 0,
            swap_fee_limit: 2_000,
            payment_hash: sha256::Hash::hash(&preimage).to_byte_array(),
            csv: 144,
            refund_pubkey: ctx.wallet().pubkey(&swap_id).unwrap(),
            claim_pubkey: service_signing.public_key(secp()),
            user_blinding_pubkey: Some(ctx.wallet().source_blinding_pubkey(&swap_id).unwrap()),
            service_blinding_pubkey: Some(service_blinding.public_key(secp())),
            claim_invoice: "lnbcrt1".into(),
            refund_address: None,
            created_at: 1,
            transitioned_at: 1,
            status,
        }
    }

    fn canceled() -> SwapInStatus {
        SwapInStatus::Canceled {
            error_code: ErrorType::UserCanceled,
            claim_privkey: SecretKey::from_slice(&SERVICE_SIGNING_KEY).unwrap(),
            cancel_message: "canceled".into(),
        }
    }

    #[test]
    fn liquid_refunds_the_deposit_and_a_stray_together() {
        let ctx = context();
        let swap = swap_in(&ctx, Chain::Liquid, canceled());
        let opening_tx = confidential_tx(&liquid_script(&swap, &ctx), &[101_000, 40_000]);

        let utxos = swap.outputs(&ctx, &opening_tx).unwrap();

        assert_eq!(utxos.len(), 2);
        assert_eq!(swap.value(&ctx, &utxos[0]).unwrap(), 101_000);
        assert_eq!(swap.value(&ctx, &utxos[1]).unwrap(), 40_000);
        let destination = liquid_destination();
        for tx_hex in [
            swap.refund_by_csv(&ctx, &utxos, &destination, 1.0).unwrap(),
            swap.refund_by_keypath(&ctx, &utxos, &destination, 1.0)
                .unwrap(),
        ] {
            let tx = anyswap_core::transaction::liquid::decode_tx_hex(&tx_hex).unwrap();
            assert_eq!(tx.input.len(), 2);
        }
    }

    #[test]
    fn liquid_skips_outputs_of_another_swap() {
        let ctx = context();
        let swap = swap_in(&ctx, Chain::Liquid, canceled());
        let other = SwapIn {
            csv: 288,
            ..swap_in(&ctx, Chain::Liquid, canceled())
        };
        let other_tx = confidential_tx(&liquid_script(&other, &ctx), &[101_000]);

        assert!(swap.outputs(&ctx, &other_tx).unwrap().is_empty());
    }

    #[test]
    fn another_swaps_output_has_no_value_here() {
        let ctx = context();
        let swap = swap_in(&ctx, Chain::Liquid, canceled());
        // A different swap id derives a different blinding key, so its outputs
        // do not unblind with this swap's.
        let other = SwapIn {
            swap_id: Uuid::from_u128(99),
            ..swap_in(&ctx, Chain::Liquid, canceled())
        };
        let other_tx = confidential_tx(&liquid_script(&other, &ctx), &[101_000]);
        let theirs = other.outputs(&ctx, &other_tx).unwrap().remove(0);

        assert!(matches!(
            swap.value(&ctx, &theirs),
            Err(SwapInError::UnknownOutput)
        ));
    }

    #[test]
    fn refunds_a_utxo_the_caller_supplied() {
        let ctx = context();
        let swap = swap_in(&ctx, Chain::Bitcoin, canceled());
        let opening_tx = bitcoin_tx(&bitcoin_script(&swap, &ctx), &[100_000]);
        let txid = anyswap_core::transaction::bitcoin::decode_tx_hex(&opening_tx)
            .unwrap()
            .compute_txid()
            .to_string();
        // Not from outputs(): the values an indexer reported, as a caller with
        // their own chain source would pass them.
        let utxos = vec![SwapUtxo::bitcoin(txid, 0, 100_000)];

        let tx_hex = swap
            .refund_by_csv(
                &ctx,
                &utxos,
                "bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080",
                1.0,
            )
            .unwrap();

        let tx = anyswap_core::transaction::bitcoin::decode_tx_hex(&tx_hex).unwrap();
        assert_eq!(tx.input.len(), 1);
        assert_eq!(swap.value(&ctx, &utxos[0]).unwrap(), 100_000);
    }

    #[test]
    fn a_keypath_refund_needs_the_revealed_claim_key() {
        let ctx = context();
        let swap = swap_in(
            &ctx,
            Chain::Liquid,
            SwapInStatus::AwaitOpeningTxConfirmation,
        );

        assert!(matches!(
            swap.refund_by_keypath(&ctx, &[], &liquid_destination(), 1.0),
            Err(SwapInError::KeypathRefundUnavailable)
        ));
    }

    #[test]
    fn a_liquid_swap_needs_a_context_set_up_for_liquid() {
        let (wallet, _) = Wallet::generate(SwapNetwork::Regtest).unwrap();
        let bitcoin_only = SwapContext::new(wallet);
        let swap = swap_in(&context(), Chain::Liquid, canceled());

        assert!(matches!(
            swap.address(&bitcoin_only),
            Err(SwapInError::LiquidNotConfigured)
        ));
    }

    #[test]
    fn a_swap_trading_another_asset_is_refused() {
        let (wallet, _) = Wallet::generate(SwapNetwork::Regtest).unwrap();
        let ctx = SwapContext::new(wallet)
            .with_liquid(&genesis_hash(), &"11".repeat(32))
            .unwrap();
        let swap = swap_in(&ctx, Chain::Liquid, canceled());

        assert!(matches!(
            swap.address(&ctx),
            Err(SwapInError::UnexpectedAsset { .. })
        ));
    }

    #[test]
    fn partition_drops_utxos_that_cannot_pay_their_own_fee() {
        let ctx = context();
        let swap = swap_in(&ctx, Chain::Bitcoin, canceled());
        let opening_tx = bitcoin_tx(&bitcoin_script(&swap, &ctx), &[100_000, 500]);

        let utxos = swap.outputs(&ctx, &opening_tx).unwrap();
        // A Bitcoin CSV refund is 138 vB, so at 10 sat/vB an input must be
        // worth more than 1_380 sat to pay for itself.
        let (economical, dust) = swap.partition_economical(&ctx, &utxos, 10.0).unwrap();

        assert_eq!(economical.len(), 1);
        assert_eq!(swap.value(&ctx, &economical[0]).unwrap(), 100_000);
        assert_eq!(dust.len(), 1);
        assert_eq!(swap.value(&ctx, &dust[0]).unwrap(), 500);
    }

    #[test]
    fn partition_reports_a_bad_fee_rate_instead_of_calling_everything_dust() {
        let ctx = context();
        let swap = swap_in(&ctx, Chain::Bitcoin, canceled());
        let opening_tx = bitcoin_tx(&bitcoin_script(&swap, &ctx), &[100_000]);
        let utxos = swap.outputs(&ctx, &opening_tx).unwrap();

        for rate in [f64::NAN, -1.0] {
            assert!(swap.partition_economical(&ctx, &utxos, rate).is_err());
        }
    }

    #[test]
    fn bitcoin_refunds_by_csv() {
        let ctx = context();
        let swap = swap_in(&ctx, Chain::Bitcoin, canceled());
        let opening_tx = bitcoin_tx(&bitcoin_script(&swap, &ctx), &[100_000]);
        let utxos = swap.outputs(&ctx, &opening_tx).unwrap();

        let tx_hex = swap
            .refund_by_csv(
                &ctx,
                &utxos,
                "bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080",
                1.0,
            )
            .unwrap();

        let tx = anyswap_core::transaction::bitcoin::decode_tx_hex(&tx_hex).unwrap();
        assert_eq!(tx.input.len(), 1);
        assert_eq!(tx.input[0].sequence.to_consensus_u32(), swap.csv);
    }

    /// The swap's Liquid script, for building the opening a counterparty funds.
    fn liquid_script(swap: &SwapIn, ctx: &SwapContext<Wallet>) -> LiquidSwapScript {
        swap.script(ctx).unwrap().liquid().unwrap().clone()
    }

    fn bitcoin_script(swap: &SwapIn, ctx: &SwapContext<Wallet>) -> BitcoinSwapScript {
        swap.script(ctx).unwrap().bitcoin().unwrap().clone()
    }

    /// A transaction paying `values` to a Bitcoin swap script.
    fn bitcoin_tx(script: &BitcoinSwapScript, values: &[u64]) -> String {
        let tx = bitcoin::Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![],
            output: values
                .iter()
                .map(|value| bitcoin::TxOut {
                    value: bitcoin::Amount::from_sat(*value),
                    script_pubkey: script.script_pubkey(),
                })
                .collect(),
        };
        anyswap_core::transaction::bitcoin::encode_tx_hex(&tx)
    }
}
