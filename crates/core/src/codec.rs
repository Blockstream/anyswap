//! String and hex codecs shared by the SDK, bindings, and plugin-facing code.
//!
//! Wire DTOs speak strings while core transaction builders require typed values.

use bitcoin::{
    Address, Txid, XOnlyPublicKey,
    secp256k1::{PublicKey, SecretKey},
};
use elements::{BlockHash as LiquidBlockHash, OutPoint as LiquidOutPoint, Txid as LiquidTxid};
use thiserror::Error;
use uuid::Uuid;

use crate::types::Outpoint;

#[derive(Debug, Error)]
#[error("invalid {what}: {message}")]
pub struct CodecError {
    what: &'static str,
    message: String,
}

fn invalid(what: &'static str, error: impl ToString) -> CodecError {
    CodecError {
        what,
        message: error.to_string(),
    }
}

pub fn parse_uuid(value: &str) -> Result<Uuid, CodecError> {
    Uuid::parse_str(value).map_err(|error| invalid("uuid", error))
}

/// Parses a hex-encoded 33-byte compressed public key.
pub fn parse_pubkey(value: &str) -> Result<PublicKey, CodecError> {
    value.parse().map_err(|error| invalid("pubkey", error))
}

/// Parses a hex-encoded public key, accepting either the 32-byte x-only or the
/// 33-byte compressed form.
pub fn parse_xonly_pubkey(value: &str) -> Result<XOnlyPublicKey, CodecError> {
    if let Ok(xonly) = value.parse::<XOnlyPublicKey>() {
        return Ok(xonly);
    }
    parse_pubkey(value).map(|pubkey| pubkey.x_only_public_key().0)
}

pub fn parse_privkey(value: &str) -> Result<SecretKey, CodecError> {
    value.parse().map_err(|error| invalid("privkey", error))
}

/// Parses a hex-encoded 32-byte hash (preimage or preimage hash).
pub fn parse_hash32(value: &str) -> Result<[u8; 32], CodecError> {
    let bytes = hex::decode(value).map_err(|error| invalid("hash", error))?;
    bytes
        .try_into()
        .map_err(|_| invalid("hash", "must be 32 bytes"))
}

/// Parses a Bitcoin txid into the chain-neutral wire representation.
pub fn parse_outpoint(txid_hex: &str, vout: u32) -> Result<Outpoint, CodecError> {
    let txid: Txid = txid_hex.parse().map_err(|error| invalid("txid", error))?;
    Ok(Outpoint::new(txid.to_string(), vout))
}

pub fn parse_bitcoin_outpoint(txid_hex: &str, vout: u32) -> Result<bitcoin::OutPoint, CodecError> {
    let txid = txid_hex
        .parse()
        .map_err(|error| invalid("Bitcoin txid", error))?;
    Ok(bitcoin::OutPoint::new(txid, vout))
}

pub fn parse_liquid_outpoint(txid_hex: &str, vout: u32) -> Result<LiquidOutPoint, CodecError> {
    let txid: LiquidTxid = txid_hex
        .parse()
        .map_err(|error| invalid("Liquid txid", error))?;
    Ok(LiquidOutPoint::new(txid, vout))
}

pub fn parse_liquid_block_hash(value: &str) -> Result<LiquidBlockHash, CodecError> {
    value
        .parse()
        .map_err(|error| invalid("Liquid block hash", error))
}

/// Parses an address. The address network is not checked against the swap
/// network; callers own that validation.
pub fn parse_address(value: &str) -> Result<Address, CodecError> {
    value
        .parse::<Address<bitcoin::address::NetworkUnchecked>>()
        .map(|address| address.assume_checked())
        .map_err(|error| invalid("address", error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chain_specific_outpoints() {
        let txid = "0000000000000000000000000000000000000000000000000000000000000001";
        assert_eq!(parse_bitcoin_outpoint(txid, 2).unwrap().vout, 2);
        assert_eq!(parse_liquid_outpoint(txid, 3).unwrap().vout, 3);
    }

    #[test]
    fn rejects_invalid_liquid_txid() {
        let error = parse_liquid_outpoint("not-a-txid", 0).unwrap_err();
        assert!(error.to_string().starts_with("invalid Liquid txid:"));
    }

    #[test]
    fn rejects_invalid_liquid_block_hash() {
        let error = parse_liquid_block_hash("not-a-block-hash").unwrap_err();
        assert!(error.to_string().starts_with("invalid Liquid block hash:"));
    }
}
