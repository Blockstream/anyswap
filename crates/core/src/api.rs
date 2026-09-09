//! HTTP API request and response types shared between the server (plugin) and SDK (client).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tracing::error;
use uuid::Uuid;

use crate::types::{Chain, SwapInStatus, SwapNetwork, SwapOutStatus, SwapType};

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct InfoResponse {
    /// Whether the Lightning node pays and receives; `false` while it runs in
    /// offline mode.
    pub online: bool,
    pub policy: Policy,
    #[serde(default)]
    pub fee_estimates: HashMap<Chain, FeeEstimate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_id: Option<Uuid>,
    /// Seconds a `quote_id` stays valid once issued.
    pub quote_ttl: u64,
    #[serde(default)]
    pub webhook: WebhookInfo,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookInfo {
    /// Whether this node accepts webhook registrations.
    pub enabled: bool,
    /// Whether a registered url must use https.
    pub https_required: bool,
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Policy {
    pub protocol_version: String,
    pub network: SwapNetwork,
    pub invoice_expiry: u64,
    pub chain: HashMap<Chain, ChainPolicyResponse>,
    pub lightning: LightningPolicyResponse,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FeeEstimate {
    pub source: u64,
    pub dest: u64,
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LightningPolicyResponse {
    pub min_amount: u64,
    pub max_amount: u64,
    pub service_ppm: u64,
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChainPolicyResponse {
    /// CSV timelock on a swap-in opening tx, in this chain's blocks.
    pub source_csv: u32,
    /// CSV timelock on a swap-out opening tx, in this chain's blocks.
    pub dest_csv: u32,
    pub confs: u32,
    pub block_time: u32,
    pub timeouts: TimeoutResponse,
    pub service_ppm: u64,
    /// Dust-guard floor: minimum receive amount as a multiple of this chain's
    /// base fee.
    pub min_base_fee_multiplier: u64,
    pub asset: HashMap<String, AssetPolicyResponse>,
}

/// Deadlines this chain enforces as a SwapIn source, in seconds. A swap left in
/// the matching status past its deadline is canceled. SwapOut has no deadline
/// here: once its opening tx confirms, the bound is the `dest_csv` timelock
/// in blocks.
#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimeoutResponse {
    /// Maximum wait for the opening tx to appear at the swap address and
    /// confirm. The claim invoice expiry may end it sooner.
    pub await_opening_tx: u64,
    /// Deadline to reveal the cooperative claim key.
    pub await_claim_key_reveal_message: u64,
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AssetPolicyResponse {
    pub asset_id: String,
    pub min_amount: u64,
    pub max_amount: u64,
}

/// Status webhook registration supplied with a swap.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookRequest {
    pub url: String,
    /// Statuses to deliver. Omitted or empty delivers every status change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statuses: Option<Vec<String>>,
    /// Send SHA256 of the swap id instead of the id itself.
    #[serde(default)]
    pub hash_swap_id: bool,
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateSwapRequest {
    pub protocol_version: String,
    pub swap_id: Uuid,
    pub user_id: String,
    pub source_chain: Option<Chain>,
    pub source_asset_id: Option<String>,
    pub dest_chain: Option<Chain>,
    pub dest_asset_id: Option<String>,
    pub receive_amount: u64,
    pub swap_fee_limit: u64,
    /// Optional `GET /v1/info` quote locking the base fee.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_id: Option<Uuid>,
    /// swap-in only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_invoice: Option<String>,
    /// swap-in and swap-chain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_refund_pubkey: Option<String>,
    /// swap-in and swap-chain, Liquid only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_blinding_pubkey: Option<String>,
    /// swap-out and swap-chain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest_claim_pubkey: Option<String>,
    /// swap-out and swap-chain, Liquid only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest_blinding_pubkey: Option<String>,
    /// swap-out and swap-chain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_claim_address: Option<String>,
    /// swap-in and swap-chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_refund_address: Option<String>,
    /// swap-out and swap-chain: preimage hash for the destination HTLC.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_hash: Option<String>,
    /// Optional webhook for swap status changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook: Option<WebhookRequest>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SwapState {
    pub protocol_version: String,
    pub swap_id: Uuid,
    pub swap_type: SwapType,
    pub status: String,
    pub receive_amount: u64,
    pub send_amount: u64,
    pub base_fee: u64,
    pub service_fee: u64,
    pub swap_fee_limit: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(ignore))]
    pub invoice_fee: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(ignore))]
    pub opening_tx_fee: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(ignore))]
    pub claim_tx_fee: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(ignore))]
    pub refund_tx_fee: Option<u64>,
    pub payment_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preimage: Option<String>,
    pub transitioned_at: u64,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_chain: Option<Chain>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_asset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_csv: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_txid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_vout: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_refund_pubkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_claim_pubkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_claim_privkey: Option<String>,
    /// Liquid: the user's blinding pubkey for the source output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_user_blinding_pubkey: Option<String>,
    /// Liquid: anyswap's blinding pubkey for the source output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_service_blinding_pubkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest_chain: Option<Chain>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest_asset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest_csv: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest_txid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest_vout: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest_refund_pubkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest_claim_pubkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest_refund_privkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_claim_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_refund_address: Option<String>,
    /// Address the server's swap-in claim pays. Internal:
    /// redacted from HTTP responses by [`Self::into_api_response`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(ignore))]
    pub server_claim_address: Option<String>,
    /// Address the server's swap-out refund pays. Internal, as above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(ignore))]
    pub server_refund_address: Option<String>,
    /// Liquid: the user's blinding pubkey for the destination output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest_user_blinding_pubkey: Option<String>,
    /// Liquid: anyswap's blinding pubkey for the destination output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest_service_blinding_pubkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_invoice: Option<String>,
    /// The `POST /v1/swap` body this swap was created from, verbatim. With the
    /// two fields below it lets a client verify the swap's terms against its own
    /// signature rather than against this response. Returned by
    /// `GET /v1/swap/{id}`, and by the listing under `with_create_request`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_request_body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_request_timestamp: Option<i64>,
    /// Hex of the 64-byte Schnorr signature over that request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_request_signature: Option<String>,
}

impl SwapState {
    /// Redacts private keys not disclosed in the current status.
    pub fn blind(mut self) -> Self {
        match self.swap_type {
            SwapType::SwapIn => self.blind_swap_in(),
            SwapType::SwapOut => self.blind_swap_out(),
            SwapType::SwapChain => {}
        }
        self
    }

    /// Redacts fields that must not be returned by public HTTP responses.
    pub fn into_api_response(mut self) -> Self {
        self = self.blind();
        self.invoice_fee = None;
        self.opening_tx_fee = None;
        self.claim_tx_fee = None;
        self.refund_tx_fee = None;
        self.server_claim_address = None;
        self.server_refund_address = None;
        self
    }

    /// Drops the signed create request. A history page reads tens of swaps at a
    /// time and each body is the size of the swap itself, so a page carries
    /// them only when asked.
    pub fn without_create_request(mut self) -> Self {
        self.create_request_body = None;
        self.create_request_timestamp = None;
        self.create_request_signature = None;
        self
    }

    fn blind_swap_in(&mut self) {
        match SwapInStatus::try_from(self.status.as_str()) {
            Ok(SwapInStatus::Canceled) => {}
            Ok(_) => self.source_claim_privkey = None,
            Err(_) => {
                error!(
                    swap_id = %self.swap_id, status = %self.status,
                    "unknown swap-in status; redacting claim privkey",
                );
                self.source_claim_privkey = None;
            }
        }
    }

    fn blind_swap_out(&mut self) {
        match SwapOutStatus::try_from(self.status.as_str()) {
            Ok(SwapOutStatus::Claimed) => {}
            Ok(_) => self.dest_refund_privkey = None,
            Err(_) => {
                error!(
                    swap_id = %self.swap_id, status = %self.status,
                    "unknown swap-out status; redacting refund privkey",
                );
                self.dest_refund_privkey = None;
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CancelRequest {
    pub cancel_message: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RevealClaimRequest {
    pub source_refund_privkey: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RevealPreimageRequest {
    pub preimage: String,
}

/// The largest page `GET /v1/swap` serves. A history page reads tens of swaps
/// at a time; the cap is what stops a client asking for the whole table in one
/// response, and it bounds how much the plugin holds in memory to answer.
pub const MAX_SWAP_LIST_LIMIT: u32 = 100;

/// Machine-readable `code` carried beside `message` in every error response,
/// so a client never infers semantics from the HTTP status.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum::Display,
    strum::IntoStaticStr,
    strum::EnumString,
)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum ErrorCode {
    Internal,
    BadRequest,
    NotImplemented,
    QuoteExpired,
    SwapNotFound,
    SwapAlreadyExists,
    /// The FSM refused a message it is past. A client treats it the same as
    /// having delivered the message.
    TransitionRejected,
    PolicyError,
    TooManyConnections,
    Unavailable,
    Forbidden,
    PayloadTooLarge,
    InsufficientLiquidity,
}

/// The body of every `4xx`/`5xx` response.
#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ErrorResponse {
    pub message: String,
    pub code: ErrorCode,
}

/// Filters for `GET /v1/swap`. They intersect, and all of them apply before
/// `limit` and `offset`, so `Pagination::total` counts what they matched. A
/// filter sent with nothing in it means the same as one left out.
#[derive(Serialize, Deserialize, Debug, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams))]
#[cfg_attr(feature = "openapi", into_params(parameter_in = Query))]
pub struct SwapListQuery {
    pub user_id: String,
    /// Page size, at most 100. A larger one is rejected, not clamped.
    #[cfg_attr(feature = "openapi", param(maximum = 100), schema(maximum = 100))]
    pub limit: u32,
    pub offset: u32,
    /// `true` keeps only active swaps, `false` only finished ones; omit for both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_active: Option<bool>,
    /// Keeps only swaps of this type: `SwapIn`, `SwapOut`, or `SwapChain`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swap_type: Option<String>,
    /// Comma-separated FSM statuses to keep, e.g. `Claimed,Refunded`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Comma-separated cancellation reasons to keep, e.g. `UserCanceled,Timeout`.
    /// Only a cancelled swap has one, so this filters out every other swap too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
    /// Keeps swaps created at or after this unix timestamp, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_after: Option<u64>,
    /// Keeps swaps created at or before this unix timestamp, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_before: Option<u64>,
    /// Keeps swaps whose `receive_amount` is at least this many sats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_amount: Option<u64>,
    /// Keeps swaps whose `receive_amount` is at most this many sats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_amount: Option<u64>,
    /// Keeps swaps where `swap_id`, `payment_hash`, `claim_invoice`,
    /// `source_txid`, `dest_txid`, or `user_claim_address` equals this value.
    /// The match ignores case but is exact, not a prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    /// `true` carries each swap's `create_request_*` fields as
    /// `GET /v1/swap/{id}` does; a page omits them otherwise.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub with_create_request: bool,
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SwapListResponse {
    pub swaps: Vec<SwapState>,
    pub pagination: Pagination,
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Pagination {
    pub total: u32,
    pub limit: u32,
    pub offset: u32,
    pub has_next: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SwapStateUpdate {
    pub swap_id: Uuid,
    pub status: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swap_state(swap_type: SwapType, status: String) -> SwapState {
        SwapState {
            protocol_version: "1.0.0".into(),
            swap_id: Uuid::nil(),
            swap_type,
            status,
            receive_amount: 100_000,
            send_amount: 101_000,
            base_fee: 1_000,
            service_fee: 0,
            swap_fee_limit: 2_000,
            invoice_fee: Some(10),
            opening_tx_fee: Some(20),
            claim_tx_fee: Some(30),
            refund_tx_fee: Some(40),
            payment_hash: "00".repeat(32),
            preimage: None,
            transitioned_at: 0,
            created_at: 0,
            error_type: None,
            cancel_message: None,
            source_chain: None,
            source_asset: None,
            source_csv: None,
            source_txid: None,
            source_vout: None,
            source_refund_pubkey: None,
            source_claim_pubkey: None,
            source_claim_privkey: Some("source-claim-secret".into()),
            source_user_blinding_pubkey: None,
            source_service_blinding_pubkey: None,
            dest_chain: None,
            dest_asset: None,
            dest_csv: None,
            dest_txid: None,
            dest_vout: None,
            dest_refund_pubkey: None,
            dest_claim_pubkey: None,
            dest_refund_privkey: Some("dest-refund-secret".into()),
            user_claim_address: None,
            user_refund_address: None,
            server_claim_address: Some("bc1qserver".into()),
            server_refund_address: Some("bc1qserver2".into()),
            dest_user_blinding_pubkey: None,
            dest_service_blinding_pubkey: None,
            claim_invoice: None,
            create_request_body: None,
            create_request_timestamp: None,
            create_request_signature: None,
        }
    }

    #[test]
    fn swap_in_claim_privkey_is_only_visible_after_cancel() {
        let active = swap_state(
            SwapType::SwapIn,
            SwapInStatus::AwaitOpeningTxConfirmation.to_string(),
        )
        .blind();
        assert_eq!(active.source_claim_privkey, None);

        let canceled = swap_state(SwapType::SwapIn, SwapInStatus::Canceled.to_string()).blind();
        assert_eq!(
            canceled.source_claim_privkey.as_deref(),
            Some("source-claim-secret")
        );
    }

    #[test]
    fn swap_out_refund_privkey_is_only_visible_after_claim() {
        let before_open = swap_state(
            SwapType::SwapOut,
            SwapOutStatus::OpeningTxBroadcast.to_string(),
        )
        .blind();
        assert_eq!(before_open.dest_refund_privkey, None);

        let awaiting_preimage = swap_state(
            SwapType::SwapOut,
            SwapOutStatus::AwaitPreimageRevealed.to_string(),
        )
        .blind();
        assert_eq!(awaiting_preimage.dest_refund_privkey, None);

        let claimed = swap_state(SwapType::SwapOut, SwapOutStatus::Claimed.to_string()).blind();
        assert_eq!(
            claimed.dest_refund_privkey.as_deref(),
            Some("dest-refund-secret")
        );
    }

    #[test]
    fn unknown_statuses_are_redacted() {
        let swap_in = swap_state(SwapType::SwapIn, "Surprise".into()).blind();
        assert_eq!(swap_in.source_claim_privkey, None);

        let swap_out = swap_state(SwapType::SwapOut, "Surprise".into()).blind();
        assert_eq!(swap_out.dest_refund_privkey, None);
    }

    #[test]
    fn api_response_hides_internal_fields() {
        let state = swap_state(
            SwapType::SwapIn,
            SwapInStatus::AwaitOpeningTxConfirmation.to_string(),
        );
        assert!(
            serde_json::to_value(&state)
                .expect("json")
                .get("server_claim_address")
                .is_some()
        );

        let json = serde_json::to_value(state.into_api_response()).expect("json");
        assert!(json.get("invoice_fee").is_none());
        assert!(json.get("opening_tx_fee").is_none());
        assert!(json.get("claim_tx_fee").is_none());
        assert!(json.get("refund_tx_fee").is_none());
        assert!(json.get("server_claim_address").is_none());
        assert!(json.get("server_refund_address").is_none());
    }
}

#[cfg(all(test, feature = "openapi"))]
mod openapi_tests {
    use utoipa::{IntoParams, PartialSchema};

    use super::*;

    fn maximum(value: &serde_json::Value) -> f64 {
        value
            .get("maximum")
            .and_then(serde_json::Value::as_f64)
            .expect("limit carries a maximum")
    }

    #[test]
    fn openapi_limit_maximum_matches_the_cap() {
        let expected = f64::from(MAX_SWAP_LIST_LIMIT);

        let schema = serde_json::to_value(SwapListQuery::schema()).expect("schema");
        assert_eq!(maximum(&schema["properties"]["limit"]), expected);

        let params = serde_json::to_value(SwapListQuery::into_params(|| None)).expect("params");
        let limit = params
            .as_array()
            .expect("params array")
            .iter()
            .find(|param| param["name"] == "limit")
            .expect("limit param");
        assert_eq!(maximum(&limit["schema"]), expected);
    }
}
