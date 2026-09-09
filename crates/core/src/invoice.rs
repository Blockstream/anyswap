//! BOLT11 invoice parsing utilities.

use bitcoin::{hashes::Hash, secp256k1::PublicKey};
use lightning_invoice::{Bolt11Invoice, RouteHint};
use thiserror::Error;

use crate::types::SwapNetwork;

#[derive(Debug, Error)]
pub enum InvoiceError {
    #[error("invalid invoice: {0}")]
    Invalid(String),

    #[error("missing amount")]
    MissingAmount,

    #[error("expiry overflow")]
    ExpiryOverflow,

    #[error("no routes found for invoice payment")]
    NoRoutesFound,
}

/// Extracts the payment hash from a BOLT11 invoice string.
pub fn payment_hash(bolt11: &str) -> Result<[u8; 32], InvoiceError> {
    let invoice = parse(bolt11)?;
    Ok(invoice.payment_hash().to_byte_array())
}

/// Extracts the amount in millisatoshis from a BOLT11 invoice string.
/// Returns an error if the invoice does not specify an amount.
pub fn amount_msat(bolt11: &str) -> Result<u64, InvoiceError> {
    let invoice = parse(bolt11)?;
    invoice
        .amount_milli_satoshis()
        .ok_or(InvoiceError::MissingAmount)
}

/// Extracts the payment secret from a BOLT11 invoice string.
pub fn payment_secret(bolt11: &str) -> Result<[u8; 32], InvoiceError> {
    let invoice = parse(bolt11)?;
    Ok(invoice.payment_secret().0)
}

/// Extracts the minimum final CLTV expiry delta from a BOLT11 invoice string.
pub fn min_final_cltv_expiry_delta(bolt11: &str) -> Result<u64, InvoiceError> {
    let invoice = parse(bolt11)?;
    Ok(invoice.min_final_cltv_expiry_delta())
}

/// Extracts the destination node id from a BOLT11 invoice.
pub fn payee_pubkey(bolt11: &str) -> Result<PublicKey, InvoiceError> {
    let invoice = parse(bolt11)?;
    Ok(invoice.get_payee_pub_key())
}

/// Extracts route hints for private channels that may not appear in public gossip.
pub fn route_hints(bolt11: &str) -> Result<Vec<RouteHint>, InvoiceError> {
    let invoice = parse(bolt11)?;
    Ok(invoice.route_hints())
}

/// Extracts the timestamp when the invoice will expire in seconds from the Unix epoch.
pub fn expiry_at(bolt11: &str) -> Result<u64, InvoiceError> {
    let invoice = parse(bolt11)?;
    invoice
        .expires_at()
        .map(|d| d.as_secs())
        .ok_or(InvoiceError::ExpiryOverflow)
}

/// Extracts the network parameter from a BOLT11 invoice string.
pub fn network(bolt11: &str) -> Result<SwapNetwork, InvoiceError> {
    let invoice = parse(bolt11)?;
    Ok(invoice.network().into())
}

fn parse(bolt11: &str) -> Result<Bolt11Invoice, InvoiceError> {
    bolt11
        .parse::<Bolt11Invoice>()
        .map_err(|e| InvoiceError::Invalid(e.to_string()))
}
