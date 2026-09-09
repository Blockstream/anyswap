//! Shared auth helpers for signing and verifying protected API requests.

use bitcoin::{
    hashes::{Hash, HashEngine, sha256},
    key::Keypair,
    secp256k1::{Message, PublicKey, Secp256k1, schnorr},
};
use thiserror::Error;

use crate::utils::now;

pub const IDENTITY_PUBKEY_HEADER: &str = "x-anyswap-identity-pubkey";
pub const TIMESTAMP_HEADER: &str = "x-anyswap-timestamp";
pub const SIGNATURE_HEADER: &str = "x-anyswap-signature";
pub const MAX_AUTH_BODY_BYTES: usize = 1024 * 1024;

/// Domain separation tag for the request digest. Bump the version suffix on any
/// change to the signed fields, their order, or their encoding.
pub const AUTH_TAG: &[u8] = b"anyswap/auth/v1";

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("missing auth field: {0}")]
    MissingField(&'static str),

    #[error("malformed pubkey")]
    MalformedPubkey,

    #[error("malformed timestamp")]
    MalformedTimestamp,

    #[error("malformed signature")]
    MalformedSignature,

    #[error("invalid timestamp")]
    InvalidTimestamp,
}

pub struct AuthParams {
    pub signature: schnorr::Signature,
    pub pubkey: PublicKey,
    pub timestamp: i64,
}

impl AuthParams {
    pub fn from_fields(pubkey: &str, timestamp: &str, signature: &str) -> Result<Self, AuthError> {
        let pubkey = Self::parse_pubkey(pubkey)?;
        let timestamp = timestamp
            .parse::<i64>()
            .map_err(|_| AuthError::MalformedTimestamp)?;
        let signature = Self::parse_signature(signature)?;

        Ok(Self {
            signature,
            pubkey,
            timestamp,
        })
    }

    pub fn from_query_params(query: &str) -> Result<Self, AuthError> {
        Self::from_fields(
            &required_query_param(query, IDENTITY_PUBKEY_HEADER)?,
            &required_query_param(query, TIMESTAMP_HEADER)?,
            &required_query_param(query, SIGNATURE_HEADER)?,
        )
    }

    pub fn validate_timestamp(&self, ttl: u64) -> Result<(), AuthError> {
        match valid_timestamp(self.timestamp, ttl) {
            Some(true) => Ok(()),
            Some(false) | None => Err(AuthError::InvalidTimestamp),
        }
    }

    fn parse_pubkey(value: &str) -> Result<PublicKey, AuthError> {
        let bytes = hex::decode(value).map_err(|_| AuthError::MalformedPubkey)?;
        PublicKey::from_slice(&bytes).map_err(|_| AuthError::MalformedPubkey)
    }

    fn parse_signature(value: &str) -> Result<schnorr::Signature, AuthError> {
        let bytes = hex::decode(value).map_err(|_| AuthError::MalformedSignature)?;
        schnorr::Signature::from_slice(&bytes).map_err(|_| AuthError::MalformedSignature)
    }
}

/// The request fields covered by the signature. Named rather than positional
/// because most of them are strings, and a transposed pair would silently sign
/// a different request than the caller meant.
///
/// `target` is the request line target, path and query together, so filters and
/// pagination are signed too. It drops the query when the auth fields travel in
/// the query string, since a signature cannot cover the value carrying it.
pub struct AuthRequest<'a> {
    pub method: &'a str,
    pub target: &'a str,
    pub body: &'a [u8],
    pub swap_id: &'a str,
    pub timestamp: i64,
}

/// A `POST /v1/swap` kept as the user signed it, so a client that lost its local
/// records can rebuild them from its own request instead of from anyswap's
/// answer. Nothing on the server reads it; it is stored and handed back.
///
/// `body` is verbatim: the digest covers its raw bytes, so a re-serialization
/// from the parsed columns would not verify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedCreateRequest {
    pub body: String,
    pub timestamp: i64,
    pub signature: Vec<u8>,
}

pub struct AuthCore;

impl AuthCore {
    /// Serializes the request into the digest preimage. Every variable length
    /// field carries a `u32` big endian length prefix, so two distinct requests
    /// can never share a serialization.
    fn serialize(req: &AuthRequest<'_>, pubkey: &PublicKey) -> Vec<u8> {
        let mut out = Vec::new();
        push_field(&mut out, req.method.as_bytes());
        push_field(&mut out, req.target.as_bytes());
        out.extend_from_slice(sha256::Hash::hash(req.body).as_byte_array());
        push_field(&mut out, req.swap_id.as_bytes());
        out.extend_from_slice(&req.timestamp.to_be_bytes());
        out.extend_from_slice(&pubkey.serialize());

        out
    }

    /// The 32 byte BIP340 message: a tagged hash over the serialized request.
    pub fn request_digest(req: &AuthRequest<'_>, pubkey: &PublicKey) -> [u8; 32] {
        tagged_hash(AUTH_TAG, &Self::serialize(req, pubkey))
    }

    pub fn validate_signature(
        digest: &[u8; 32],
        signature: &schnorr::Signature,
        pubkey: &PublicKey,
    ) -> bool {
        let secp = Secp256k1::verification_only();
        let message = Message::from_digest(*digest);
        let x_only_pubkey = pubkey.x_only_public_key().0;

        secp.verify_schnorr(signature, &message, &x_only_pubkey)
            .is_ok()
    }

    pub fn validate_pubkey(pubkey: &PublicKey, user_id: &str) -> bool {
        sha256::Hash::hash(&pubkey.serialize()).to_string() == user_id
    }

    pub fn sign_request(req: &AuthRequest<'_>, identity_keypair: &Keypair) -> schnorr::Signature {
        let secp = Secp256k1::new();
        let digest = Self::request_digest(req, &identity_keypair.public_key());

        secp.sign_schnorr(&Message::from_digest(digest), identity_keypair)
    }
}

fn push_field(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u32).to_be_bytes());
    out.extend_from_slice(field);
}

/// BIP340 style tagged hash: `SHA256(SHA256(tag) || SHA256(tag) || msg)`.
fn tagged_hash(tag: &[u8], msg: &[u8]) -> [u8; 32] {
    let tag_hash = sha256::Hash::hash(tag);
    let mut engine = sha256::Hash::engine();
    engine.input(tag_hash.as_byte_array());
    engine.input(tag_hash.as_byte_array());
    engine.input(msg);

    sha256::Hash::from_engine(engine).to_byte_array()
}

pub fn valid_timestamp(timestamp: i64, ttl: u64) -> Option<bool> {
    let now = now();
    let timestamp = u64::try_from(timestamp).ok()?;

    // accept drift up to ttl in either direction to tolerate clock skew
    Some(now.abs_diff(timestamp) <= ttl)
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key).then(|| value.to_owned())
    })
}

fn required_query_param(query: &str, key: &'static str) -> Result<String, AuthError> {
    query_param(query, key).ok_or(AuthError::MissingField(key))
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{Keypair, SecretKey};

    use super::*;

    const TEST_TTL: u64 = 60;
    const CANCEL_BODY: &[u8] = br#"{"cancel_message":"test"}"#;

    fn test_keypair() -> Keypair {
        let secp = Secp256k1::new();
        let secret = SecretKey::from_slice(&[1; 32]).unwrap();
        Keypair::from_secret_key(&secp, &secret)
    }

    fn cancel_request() -> AuthRequest<'static> {
        AuthRequest {
            method: "POST",
            target: "/v1/swap/swap-1/cancel",
            body: CANCEL_BODY,
            swap_id: "swap-1",
            timestamp: 42,
        }
    }

    #[test]
    fn query_param_finds_named_value() {
        assert_eq!(
            query_param("limit=50&user_id=alice&offset=0", "user_id"),
            Some("alice".to_string())
        );
        assert_eq!(query_param("limit=50", "user_id"), None);
    }

    #[test]
    fn auth_params_parse_query_params() {
        let keypair = test_keypair();
        let pubkey = keypair.public_key();
        let signature = schnorr::Signature::from_slice(&[2; 64]).unwrap();
        let query = format!(
            "{}={}&{}=42&{}={}",
            IDENTITY_PUBKEY_HEADER,
            hex::encode(pubkey.serialize()),
            TIMESTAMP_HEADER,
            SIGNATURE_HEADER,
            hex::encode(signature.as_ref())
        );

        let params = AuthParams::from_query_params(&query).unwrap();

        assert_eq!(params.pubkey, pubkey);
        assert_eq!(params.timestamp, 42);
        assert_eq!(params.signature, signature);
    }

    #[test]
    fn valid_timestamp_accepts_current_time() {
        let now = now() as i64;

        assert_eq!(valid_timestamp(now, TEST_TTL), Some(true));
    }

    #[test]
    fn valid_timestamp_rejects_old_time() {
        let now = now() as i64;
        let old = now - TEST_TTL as i64 - 1;

        assert_eq!(valid_timestamp(old, TEST_TTL), Some(false));
    }

    #[test]
    fn sign_request_produces_a_verifiable_signature() {
        let keypair = test_keypair();
        let signature = AuthCore::sign_request(&cancel_request(), &keypair);
        let digest = AuthCore::request_digest(&cancel_request(), &keypair.public_key());

        assert!(AuthCore::validate_signature(
            &digest,
            &signature,
            &keypair.public_key()
        ));
    }

    #[test]
    fn request_digest_is_a_tagged_hash_over_length_prefixed_fields() {
        let pubkey = test_keypair().public_key();
        let req = cancel_request();

        let mut preimage = Vec::new();
        preimage.extend_from_slice(&(req.method.len() as u32).to_be_bytes());
        preimage.extend_from_slice(req.method.as_bytes());
        preimage.extend_from_slice(&(req.target.len() as u32).to_be_bytes());
        preimage.extend_from_slice(req.target.as_bytes());
        preimage.extend_from_slice(sha256::Hash::hash(CANCEL_BODY).as_byte_array());
        preimage.extend_from_slice(&(req.swap_id.len() as u32).to_be_bytes());
        preimage.extend_from_slice(req.swap_id.as_bytes());
        preimage.extend_from_slice(&42i64.to_be_bytes());
        preimage.extend_from_slice(&pubkey.serialize());

        assert_eq!(
            AuthCore::request_digest(&req, &pubkey),
            tagged_hash(b"anyswap/auth/v1", &preimage)
        );
    }

    #[test]
    fn request_digest_separates_target_from_swap_id() {
        let pubkey = test_keypair().public_key();
        let shifted = AuthRequest {
            target: "/v1/swap/a",
            swap_id: "bc",
            ..cancel_request()
        };
        let original = AuthRequest {
            target: "/v1/swap/ab",
            swap_id: "c",
            ..cancel_request()
        };

        assert_ne!(
            AuthCore::request_digest(&original, &pubkey),
            AuthCore::request_digest(&shifted, &pubkey)
        );
    }

    #[test]
    fn request_digest_separates_swap_id_from_timestamp() {
        let pubkey = test_keypair().public_key();
        let shifted = AuthRequest {
            swap_id: "swap-",
            timestamp: 142,
            ..cancel_request()
        };
        let original = AuthRequest {
            swap_id: "swap-1",
            timestamp: 42,
            ..cancel_request()
        };

        assert_ne!(
            AuthCore::request_digest(&original, &pubkey),
            AuthCore::request_digest(&shifted, &pubkey)
        );
    }

    #[test]
    fn request_digest_covers_the_query_string() {
        let pubkey = test_keypair().public_key();
        let listed = AuthRequest {
            target: "/v1/swap?user_id=alice&limit=20",
            ..cancel_request()
        };
        let widened = AuthRequest {
            target: "/v1/swap?user_id=alice&limit=500",
            ..cancel_request()
        };

        assert_ne!(
            AuthCore::request_digest(&listed, &pubkey),
            AuthCore::request_digest(&widened, &pubkey)
        );
    }

    #[test]
    fn valid_timestamp_accepts_future_within_ttl() {
        let now = now() as i64;
        let future = now + TEST_TTL as i64 - 1;

        assert_eq!(valid_timestamp(future, TEST_TTL), Some(true));
    }

    #[test]
    fn valid_timestamp_rejects_future_beyond_ttl() {
        let now = now() as i64;
        let future = now + TEST_TTL as i64 + 5;

        assert_eq!(valid_timestamp(future, TEST_TTL), Some(false));
    }

    #[test]
    fn valid_timestamp_rejects_negative_time() {
        assert_eq!(valid_timestamp(-1, TEST_TTL), None);
    }
}
