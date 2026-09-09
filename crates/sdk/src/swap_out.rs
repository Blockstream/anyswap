//! Typed client-side view of a swap-out.
//!
//! [`SwapOut`] is the parsed form of the wire [`SwapState`]: hex fields become
//! keys and outpoints, and per-status fields move into [`SwapOutStatus`]
//! variants so the consumer sees exactly what is available in each state.
//! Created via `TryFrom<SwapState>` on the polling response.
//!
//! A swap-out moves funds from Lightning to on-chain. The user holds the
//! preimage and pays a held `claim_invoice`; the server then funds the
//! destination opening transaction. The user is the claim side, the server is
//! the refund side - the mirror of [`swap_in`](crate::swap_in).

pub use anyswap_core::types::{ErrorType, SwapOutStatus as Status};
use anyswap_core::{
    api::SwapState,
    transaction::TxError,
    types::{Chain, Outpoint, SwapType},
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
pub enum SwapOutError {
    #[error("expected swap type SwapOut, got {0}")]
    UnexpectedSwapType(SwapType),

    #[error("unknown status: {0}")]
    UnknownStatus(String),

    #[error("unknown error type: {0}")]
    UnknownErrorType(String),

    #[error("missing field: {0}")]
    MissingField(&'static str),

    #[error("a keypath claim needs the refund key the server reveals once the preimage is out")]
    KeypathClaimUnavailable,

    #[error("no opening outpoint in current status")]
    NoOutpoint,

    #[error("expected opening tx {expected}, got {found}")]
    UnexpectedTxid { expected: String, found: String },

    #[error("opening output value {found_sat} sat does not match expected {expected_sat} sat")]
    AmountMismatch { expected_sat: u64, found_sat: u64 },

    #[error("server swap fee {swap_fee_sat} sat exceeds agreed limit {swap_fee_limit_sat} sat")]
    SwapFeeExceedsLimit {
        swap_fee_sat: u64,
        swap_fee_limit_sat: u64,
    },

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

    #[error(transparent)]
    Utxo(#[from] UtxoError),

    #[error(transparent)]
    Tx(#[from] TxError),

    #[error(transparent)]
    Codec(#[from] CodecError),
}

impl From<ScriptError> for SwapOutError {
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

/// A swap-out: the user pays a held `claim_invoice` and the server locks the
/// proceeds into the swap script on-chain. The user claims that output with the
/// preimage; the server refunds it after the CSV timeout if the user never
/// claims. The user holds the claim key, the server holds the refund key.
#[derive(Debug, Clone)]
pub struct SwapOut {
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
    /// User's pubkey in the swap script (claim side).
    pub claim_pubkey: PublicKey,
    /// Server's pubkey in the swap script (refund side).
    pub refund_pubkey: PublicKey,
    /// User's ECDH blinding pubkey for the Liquid opening output.
    pub user_blinding_pubkey: Option<PublicKey>,
    /// Server's ECDH blinding pubkey for the Liquid opening output.
    pub service_blinding_pubkey: Option<PublicKey>,
    /// User's on-chain destination for a successful claim.
    pub claim_address: String,
    /// Held invoice the user pays to start the swap.
    pub claim_invoice: String,
    pub created_at: u64,
    pub transitioned_at: u64,
    pub status: SwapOutStatus,
}

/// Server-reported swap-out lifecycle state with the fields available in it.
///
/// Normal flow: [`AwaitClaimInvoicePay`](Self::AwaitClaimInvoicePay) ->
/// [`OpeningTxBroadcast`](Self::OpeningTxBroadcast) ->
/// [`AwaitPreimageRevealed`](Self::AwaitPreimageRevealed) ->
/// [`Claimed`](Self::Claimed).
///
/// If the user never claims, the server refunds the destination output after
/// the CSV timeout: [`AwaitRefundTxConfirmation`](Self::AwaitRefundTxConfirmation)
/// -> [`Refunded`](Self::Refunded). Cancellation before the opening tx is
/// broadcast results in [`Canceled`](Self::Canceled).
#[derive(Debug, Clone, strum::Display, strum::IntoStaticStr)]
pub enum SwapOutStatus {
    /// Server is waiting for the user to pay the held `claim_invoice`.
    AwaitClaimInvoicePay,
    /// Invoice paid; the server is broadcasting the destination opening tx.
    /// No actionable outpoint yet - wait for
    /// [`AwaitPreimageRevealed`](Self::AwaitPreimageRevealed).
    OpeningTxBroadcast,
    /// Opening tx broadcast at `outpoint`; the user claims once it has the
    /// required confirmations, either by revealing the preimage off-chain for a
    /// cooperative keypath claim (via
    /// [`HttpClient::reveal_preimage`](crate::client::HttpClient::reveal_preimage),
    /// reaching [`Claimed`](Self::Claimed)) or unilaterally on-chain via
    /// [`SwapOut::claim_by_preimage`].
    AwaitPreimageRevealed { outpoint: Outpoint },
    /// The user did not claim in time; the server is refunding the destination
    /// output after the CSV timeout. The user can still race a unilateral
    /// [`SwapOut::claim_by_preimage`] until the refund confirms.
    AwaitRefundTxConfirmation { outpoint: Outpoint },
    /// Terminal: the preimage was revealed and the held invoice settled. The
    /// server reveals its `refund_privkey`; if the preimage was revealed
    /// off-chain, finish the cooperative claim via [`SwapOut::claim_by_keypath`].
    Claimed {
        outpoint: Outpoint,
        refund_privkey: SecretKey,
    },
    /// Terminal: canceled before the opening tx was broadcast. `error_code` is
    /// the machine-readable reason. No on-chain funds at risk; the held invoice is
    /// failed back over Lightning.
    Canceled {
        error_code: ErrorType,
        cancel_message: String,
    },
    /// Terminal: the destination output was refunded to the server after the
    /// CSV timeout and the held invoice failed back.
    Refunded { outpoint: Outpoint },
}

impl SwapOutStatus {
    /// Opening outpoint, if the server has broadcast the deposit in the current
    /// state.
    pub fn outpoint(&self) -> Option<&Outpoint> {
        match self {
            Self::AwaitClaimInvoicePay | Self::OpeningTxBroadcast | Self::Canceled { .. } => None,
            Self::AwaitPreimageRevealed { outpoint }
            | Self::AwaitRefundTxConfirmation { outpoint }
            | Self::Claimed { outpoint, .. }
            | Self::Refunded { outpoint } => Some(outpoint),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Claimed { .. } | Self::Canceled { .. } | Self::Refunded { .. }
        )
    }
}

/// The user is the claim side of a swap-out, so these build claims only: a
/// unilateral one that reveals the preimage on chain, and a cooperative one once
/// the server reveals its refund key. The per-swap keys and the preimage come
/// from the context's wallet, so a caller never handles a private key.
impl SwapOut {
    /// The address the server's opening transaction pays.
    pub fn address<W: SwapWallet>(&self, ctx: &SwapContext<W>) -> Result<String, SwapOutError> {
        Ok(self.script(ctx)?.address())
    }

    /// Checks the server's opening transaction and returns the output to claim.
    ///
    /// Call this before releasing the preimage, whether by
    /// [`claim_by_preimage`](Self::claim_by_preimage) or by
    /// [`HttpClient::reveal_preimage`](crate::client::HttpClient::reveal_preimage).
    /// The opening does not exist until the held invoice is paid, so paying is
    /// not the moment this guards; the preimage is, because releasing it settles
    /// the invoice whatever the opening turned out to be.
    ///
    /// The server funds this output, so its report is not enough on its own: a
    /// dust value under the right script would otherwise pass unnoticed. This
    /// checks the transaction against the outpoint the swap recorded, the output
    /// against the agreed script and asset, its value against `receive_amount`,
    /// and the server's fee against the limit the user accepted. That anchors
    /// trust in the chain rather than in what the server says.
    pub fn verify_opening<W: SwapWallet>(
        &self,
        ctx: &SwapContext<W>,
        tx_hex: &str,
    ) -> Result<SwapUtxo, SwapOutError> {
        let outpoint = self.status.outpoint().ok_or(SwapOutError::NoOutpoint)?;
        let tx = Tx::decode(self.chain, tx_hex)?;
        if tx.txid() != outpoint.txid {
            return Err(SwapOutError::UnexpectedTxid {
                expected: outpoint.txid.clone(),
                found: tx.txid(),
            });
        }
        let (prevout, value) = self.script(ctx)?.output_at(&tx, outpoint.vout)?;
        let swap_fee = self.base_fee.saturating_add(self.service_fee);
        if swap_fee > self.swap_fee_limit {
            return Err(SwapOutError::SwapFeeExceedsLimit {
                swap_fee_sat: swap_fee,
                swap_fee_limit_sat: self.swap_fee_limit,
            });
        }
        if value != self.receive_amount {
            return Err(SwapOutError::AmountMismatch {
                expected_sat: self.receive_amount,
                found_sat: value,
            });
        }
        Ok(prevout)
    }

    /// Claims `utxos` by revealing the preimage on chain, which is also how the
    /// server learns it and settles the held invoice. This is the fallback when
    /// the server does not cooperate. `destination` defaults to the swap's
    /// `claim_address`. Returns the signed transaction as hex.
    pub fn claim_by_preimage<W: SwapWallet>(
        &self,
        ctx: &SwapContext<W>,
        utxos: &[SwapUtxo],
        destination: Option<&str>,
        fee_rate: f64,
    ) -> Result<String, SwapOutError> {
        let claim_privkey = self.claim_privkey(ctx)?;
        let preimage = ctx
            .wallet()
            .preimage(&self.swap_id)
            .map_err(|error| SwapOutError::Wallet(error.to_string()))?;
        Ok(self.script(ctx)?.claim_by_preimage(
            utxos,
            claim_privkey,
            preimage,
            &self.destination(destination),
            fee_rate,
        )?)
    }

    /// Claims `utxos` cooperatively, once the server has revealed its refund key
    /// in exchange for the preimage. Cheaper than the preimage path and it keeps
    /// the preimage off chain. `destination` defaults to the swap's
    /// `claim_address`. Returns the signed transaction as hex.
    pub fn claim_by_keypath<W: SwapWallet>(
        &self,
        ctx: &SwapContext<W>,
        utxos: &[SwapUtxo],
        destination: Option<&str>,
        fee_rate: f64,
    ) -> Result<String, SwapOutError> {
        let SwapOutStatus::Claimed { refund_privkey, .. } = &self.status else {
            return Err(SwapOutError::KeypathClaimUnavailable);
        };
        let claim_privkey = self.claim_privkey(ctx)?;
        Ok(self.script(ctx)?.keypath(
            utxos,
            claim_privkey,
            *refund_privkey,
            &self.destination(destination),
            fee_rate,
        )?)
    }

    /// The value of `utxo` in sats. Bitcoin states it on chain; a Liquid value
    /// is blinded, and this unblinds it with the swap's own key.
    pub fn value<W: SwapWallet>(
        &self,
        ctx: &SwapContext<W>,
        utxo: &SwapUtxo,
    ) -> Result<u64, SwapOutError> {
        self.script(ctx)?
            .value(utxo)
            .ok_or(SwapOutError::UnknownOutput)
    }

    /// The terms the opening script is built from.
    pub fn terms(&self) -> Terms {
        Terms {
            swap_id: self.swap_id,
            side: Side::Dest,
            chain: self.chain,
            asset: self.asset.clone(),
            csv: self.csv,
            payment_hash: self.payment_hash,
            server_pubkey: self.refund_pubkey,
            server_blinding_pubkey: self.service_blinding_pubkey,
        }
    }

    fn script<W: SwapWallet>(&self, ctx: &SwapContext<W>) -> Result<SwapScript, SwapOutError> {
        Ok(ctx.script(&self.terms())?)
    }

    /// The user's key in the swap script.
    fn claim_privkey<W: SwapWallet>(
        &self,
        ctx: &SwapContext<W>,
    ) -> Result<SecretKey, SwapOutError> {
        ctx.wallet()
            .privkey(&self.swap_id)
            .map_err(|error| SwapOutError::Wallet(error.to_string()))
    }

    fn destination(&self, destination: Option<&str>) -> String {
        destination.unwrap_or(&self.claim_address).to_string()
    }
}

impl TryFrom<SwapState> for SwapOut {
    type Error = SwapOutError;

    fn try_from(s: SwapState) -> Result<Self, SwapOutError> {
        if s.swap_type != SwapType::SwapOut {
            return Err(SwapOutError::UnexpectedSwapType(s.swap_type));
        }

        let status: Status = s
            .status
            .parse()
            .map_err(|_| SwapOutError::UnknownStatus(s.status.clone()))?;

        let status = match status {
            Status::AwaitClaimInvoicePay => SwapOutStatus::AwaitClaimInvoicePay,
            Status::OpeningTxBroadcast => SwapOutStatus::OpeningTxBroadcast,
            Status::AwaitPreimageRevealed => SwapOutStatus::AwaitPreimageRevealed {
                outpoint: dest_outpoint(&s)?,
            },
            Status::AwaitRefundTxConfirmation => SwapOutStatus::AwaitRefundTxConfirmation {
                outpoint: dest_outpoint(&s)?,
            },
            Status::Claimed => SwapOutStatus::Claimed {
                outpoint: dest_outpoint(&s)?,
                refund_privkey: codec::parse_privkey(&require(
                    s.dest_refund_privkey.clone(),
                    "dest_refund_privkey",
                )?)?,
            },
            Status::Canceled => {
                let raw = require(s.error_type.clone(), "error_type")?;
                SwapOutStatus::Canceled {
                    error_code: raw
                        .parse()
                        .map_err(|_| SwapOutError::UnknownErrorType(raw.clone()))?,
                    cancel_message: require(s.cancel_message.clone(), "cancel_message")?,
                }
            }
            Status::Refunded => SwapOutStatus::Refunded {
                outpoint: dest_outpoint(&s)?,
            },
        };
        let user_blinding_pubkey = s
            .dest_user_blinding_pubkey
            .as_deref()
            .map(codec::parse_pubkey)
            .transpose()?;
        let service_blinding_pubkey = s
            .dest_service_blinding_pubkey
            .as_deref()
            .map(codec::parse_pubkey)
            .transpose()?;

        Ok(SwapOut {
            swap_id: s.swap_id,
            protocol_version: s.protocol_version,
            chain: require(s.dest_chain, "dest_chain")?,
            asset: require(s.dest_asset, "dest_asset")?,
            receive_amount: s.receive_amount,
            send_amount: s.send_amount,
            base_fee: s.base_fee,
            service_fee: s.service_fee,
            swap_fee_limit: s.swap_fee_limit,
            payment_hash: codec::parse_hash32(&s.payment_hash)?,
            csv: require(s.dest_csv, "dest_csv")?,
            claim_pubkey: codec::parse_pubkey(&require(s.dest_claim_pubkey, "dest_claim_pubkey")?)?,
            refund_pubkey: codec::parse_pubkey(&require(
                s.dest_refund_pubkey,
                "dest_refund_pubkey",
            )?)?,
            user_blinding_pubkey,
            service_blinding_pubkey,
            claim_address: require(s.user_claim_address, "user_claim_address")?,
            claim_invoice: require(s.claim_invoice, "claim_invoice")?,
            created_at: s.created_at,
            transitioned_at: s.transitioned_at,
            status,
        })
    }
}

fn require<T>(opt: Option<T>, field: &'static str) -> Result<T, SwapOutError> {
    opt.ok_or(SwapOutError::MissingField(field))
}

fn dest_outpoint(s: &SwapState) -> Result<Outpoint, SwapOutError> {
    Ok(codec::parse_outpoint(
        require(s.dest_txid.as_deref(), "dest_txid")?,
        require(s.dest_vout, "dest_vout")?,
    )?)
}

#[cfg(test)]
mod tests {
    use anyswap_core::{
        secp,
        transaction::liquid::decode_tx_hex,
        types::{Chain, SwapNetwork},
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

    /// A Liquid swap-out whose opening is recorded at `outpoint`.
    fn swap_out(ctx: &SwapContext<Wallet>, status: SwapOutStatus) -> SwapOut {
        let swap_id = Uuid::from_u128(12);
        let service_signing = SecretKey::from_slice(&SERVICE_SIGNING_KEY).unwrap();
        let service_blinding = SecretKey::from_slice(&SERVICE_BLINDING_KEY).unwrap();
        let preimage = ctx.wallet().preimage(&swap_id).unwrap();
        SwapOut {
            swap_id,
            protocol_version: "2.0.0".into(),
            chain: Chain::Liquid,
            asset: liquid_asset(),
            receive_amount: 100_000,
            send_amount: 101_000,
            base_fee: 1_000,
            service_fee: 0,
            swap_fee_limit: 2_000,
            payment_hash: sha256::Hash::hash(&preimage).to_byte_array(),
            csv: 144,
            claim_pubkey: ctx.wallet().pubkey(&swap_id).unwrap(),
            refund_pubkey: service_signing.public_key(secp()),
            user_blinding_pubkey: Some(ctx.wallet().dest_blinding_pubkey(&swap_id).unwrap()),
            service_blinding_pubkey: Some(service_blinding.public_key(secp())),
            claim_address: liquid_destination(),
            claim_invoice: "lnbcrt1".into(),
            created_at: 1,
            transitioned_at: 1,
            status,
        }
    }

    fn awaiting(txid: &str) -> SwapOutStatus {
        SwapOutStatus::AwaitPreimageRevealed {
            outpoint: Outpoint::new(txid, 0),
        }
    }

    /// Builds the opening the server would fund, then re-points the swap at it.
    fn opening(ctx: &SwapContext<Wallet>, value: u64) -> (SwapOut, String) {
        let swap = swap_out(ctx, awaiting("00"));
        let script = swap.script(ctx).unwrap();
        let tx_hex = confidential_tx(script.liquid().unwrap(), &[value]);
        let txid = decode_tx_hex(&tx_hex).unwrap().txid().to_string();
        (swap_out(ctx, awaiting(&txid)), tx_hex)
    }

    #[test]
    fn verify_opening_returns_the_output_to_claim() {
        let ctx = context();
        let (swap, tx_hex) = opening(&ctx, 100_000);

        let utxo = swap.verify_opening(&ctx, &tx_hex).unwrap();

        assert_eq!(swap.value(&ctx, &utxo).unwrap(), 100_000);
        assert_eq!(utxo.vout, 0);
        let claim = swap.claim_by_preimage(&ctx, &[utxo], None, 1.0).unwrap();
        assert_eq!(decode_tx_hex(&claim).unwrap().input.len(), 1);
    }

    #[test]
    fn verify_opening_rejects_a_short_payout() {
        let ctx = context();
        let (swap, tx_hex) = opening(&ctx, 99_999);

        assert!(matches!(
            swap.verify_opening(&ctx, &tx_hex),
            Err(SwapOutError::AmountMismatch {
                expected_sat: 100_000,
                found_sat: 99_999,
            })
        ));
    }

    #[test]
    fn verify_opening_rejects_a_transaction_the_swap_did_not_record() {
        let ctx = context();
        let (_, tx_hex) = opening(&ctx, 100_000);
        let swap = swap_out(&ctx, awaiting("11".repeat(32).as_str()));

        assert!(matches!(
            swap.verify_opening(&ctx, &tx_hex),
            Err(SwapOutError::UnexpectedTxid { .. })
        ));
    }

    #[test]
    fn verify_opening_rejects_a_fee_above_the_agreed_limit() {
        let ctx = context();
        let (mut swap, tx_hex) = opening(&ctx, 100_000);
        swap.base_fee = 3_000;

        assert!(matches!(
            swap.verify_opening(&ctx, &tx_hex),
            Err(SwapOutError::SwapFeeExceedsLimit {
                swap_fee_sat: 3_000,
                swap_fee_limit_sat: 2_000,
            })
        ));
    }

    #[test]
    fn a_keypath_claim_needs_the_revealed_refund_key() {
        let ctx = context();
        let (swap, tx_hex) = opening(&ctx, 100_000);
        let utxo = swap.verify_opening(&ctx, &tx_hex).unwrap();

        assert!(matches!(
            swap.claim_by_keypath(&ctx, std::slice::from_ref(&utxo), None, 1.0),
            Err(SwapOutError::KeypathClaimUnavailable)
        ));

        let claimed = SwapOut {
            status: SwapOutStatus::Claimed {
                outpoint: utxo.outpoint(),
                refund_privkey: SecretKey::from_slice(&SERVICE_SIGNING_KEY).unwrap(),
            },
            ..swap
        };
        let tx_hex = claimed.claim_by_keypath(&ctx, &[utxo], None, 1.0).unwrap();
        assert_eq!(decode_tx_hex(&tx_hex).unwrap().input.len(), 1);
    }
}
