//! Locks the on-chain side of the swap, with taproot implementations in
//! [`bitcoin`] and [`liquid`].

use anyhow::Error;

use crate::musig;

pub mod bitcoin;
pub mod liquid;

pub use self::{
    bitcoin::BitcoinSwapScript,
    liquid::{LiquidSwapScript, derive_liquid_blinding_key},
};

pub(super) fn validate_parameters(
    claim_pubkey: ::bitcoin::XOnlyPublicKey,
    refund_pubkey: ::bitcoin::XOnlyPublicKey,
    csv_blocks: u32,
) -> Result<::bitcoin::XOnlyPublicKey, Error> {
    if claim_pubkey == refund_pubkey {
        return Err(Error::msg("claim and refund pubkeys must differ"));
    }
    if !(1..=0xFFFF).contains(&csv_blocks) {
        return Err(Error::msg(format!(
            "csv_blocks must be in 1..=65535, got {}",
            csv_blocks
        )));
    }

    Ok(musig::aggregate_xonly(claim_pubkey, refund_pubkey)?)
}
