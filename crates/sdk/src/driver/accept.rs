//! Fixing the terms: a quote from `/info`, the acceptance checks a swap has to
//! pass before it is driven, and restoring one from our own signed request.

use std::collections::HashSet;

use anyswap_core::{
    api::{CreateSwapRequest, InfoResponse, MAX_SWAP_LIST_LIMIT, SwapListQuery, SwapState},
    auth::{AuthCore, AuthRequest},
    fee, invoice,
    types::{Chain, NATIVE_ASSET, SwapNetwork, SwapType},
    utils::validate_user_address,
};
use bitcoin::secp256k1::{PublicKey, schnorr::Signature};
use elements::AssetId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    Driver, MaybeSend, MaybeSync, Swap,
    error::{DriverError, Refusal},
    record::{Endpoint, Sent, Status, SwapRecord, Timestamp},
    tick::{ServerStatus, driven, poll},
    wallet_error,
};
use crate::{
    swap_in::{SwapIn, SwapInStatus},
    swap_out::{SwapOut, SwapOutStatus},
    swap_script::Side,
    wallet::{SwapWallet, SwapWalletExt},
};

/// Lightning counts CLTV in Bitcoin blocks whatever chain the opening is on.
const LIGHTNING_BLOCK_TIME: u64 = 600;

/// `GET /v1/info` reduced to two ends and one amount, with the `quote_id`
/// that locks it. An end is Lightning when its chain is `None`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Quote {
    pub source_chain: Option<Chain>,
    pub source_asset_id: Option<String>,
    pub dest_chain: Option<Chain>,
    pub dest_asset_id: Option<String>,
    pub receive_amount: u64,
    pub send_amount: u64,
    pub base_fee: u64,
    pub service_fee: u64,
    pub fee: u64,
    pub quote_id: Uuid,
    pub expires_at: Timestamp,
    pub(crate) protocol_version: String,
}

pub(crate) fn quote(
    info: InfoResponse,
    network: SwapNetwork,
    policy_asset: Option<AssetId>,
    source: Option<Chain>,
    dest: Option<Chain>,
    receive_amount: u64,
    now: Timestamp,
) -> Result<Quote, DriverError> {
    if info.policy.network != network {
        return Err(Refusal::Network {
            expected: network,
            found: info.policy.network,
        }
        .into());
    }
    let quote_id = info
        .quote_id
        .ok_or(DriverError::NotQuotable("no chain is quotable"))?;
    let (chain, side) = match (source, dest) {
        (Some(chain), None) => (chain, Side::Source),
        (None, Some(chain)) => (chain, Side::Dest),
        (Some(_), Some(_)) => return Err(DriverError::Unsupported(SwapType::SwapChain)),
        (None, None) => return Err(DriverError::NotQuotable("neither end is on chain")),
    };
    let policy = info
        .policy
        .chain
        .get(&chain)
        .ok_or(DriverError::NotQuotable("chain not offered"))?;
    let asset = match chain {
        Chain::Bitcoin => NATIVE_ASSET.to_string(),
        Chain::Liquid => policy_asset
            .ok_or(Refusal::LiquidNotConfigured)?
            .to_string(),
    };
    let asset_policy = policy
        .asset
        .values()
        .find(|entry| entry.asset_id == asset)
        .ok_or(DriverError::NotQuotable("asset not offered"))?;
    let estimate = info
        .fee_estimates
        .get(&chain)
        .ok_or(DriverError::NotQuotable("no fee estimate"))?;
    let lightning = &info.policy.lightning;
    let (base_fee, service_ppm, min, max) = match side {
        Side::Source => (
            estimate.source,
            lightning.service_ppm,
            lightning.min_amount,
            lightning.max_amount,
        ),
        Side::Dest => (
            estimate.dest,
            policy.service_ppm,
            asset_policy.min_amount,
            asset_policy.max_amount,
        ),
    };
    let min = min.max(base_fee.saturating_mul(policy.min_base_fee_multiplier));
    if receive_amount < min || receive_amount > max {
        return Err(DriverError::AmountOutOfRange { min, max });
    }
    let service_fee = fee::service_fee(receive_amount, service_ppm);
    let fee = base_fee.saturating_add(service_fee);
    let send_amount = receive_amount
        .checked_add(fee)
        .ok_or(DriverError::AmountOutOfRange { min, max })?;
    let (source_asset_id, dest_asset_id) = match side {
        Side::Source => (Some(asset), None),
        Side::Dest => (None, Some(asset)),
    };
    Ok(Quote {
        source_chain: source,
        source_asset_id,
        dest_chain: dest,
        dest_asset_id,
        receive_amount,
        send_amount,
        base_fee,
        service_fee,
        fee,
        quote_id,
        expires_at: now + info.quote_ttl,
        protocol_version: info.policy.protocol_version,
    })
}

/// The side, chain and asset a request asks for.
fn asked(req: &CreateSwapRequest) -> Result<(Side, Chain, &str), Refusal> {
    let (side, chain, asset) = match (req.source_chain, req.dest_chain) {
        (Some(chain), None) => (Side::Source, chain, &req.source_asset_id),
        (None, Some(chain)) => (Side::Dest, chain, &req.dest_asset_id),
        _ => return Err(Refusal::Malformed("neither a SwapIn nor a SwapOut".into())),
    };
    let asset = asset
        .as_deref()
        .ok_or_else(|| Refusal::Malformed("no asset".into()))?;
    Ok((side, chain, asset))
}

impl<W: SwapWallet + MaybeSend + MaybeSync + 'static> Driver<W> {
    /// Runs the acceptance checks against the request, the yardstick both at
    /// creation and on restore, and freezes the terms. After this the server's
    /// copy of them is never read again.
    pub(crate) fn accept(
        &self,
        req: &CreateSwapRequest,
        started_at: Timestamp,
        state: SwapState,
    ) -> Result<(SwapRecord, ServerStatus), DriverError> {
        let (side, chain, _) = asked(req)?;
        let cc = &self.chain(chain)?.config;
        let (ctx, config) = (&self.core.ctx, &self.config);
        let swap_type = match side {
            Side::Source => SwapType::SwapIn,
            Side::Dest => SwapType::SwapOut,
        };
        if state.swap_type != swap_type {
            return Err(Refusal::SwapType(state.swap_type).into());
        }
        common(req, side, &state)?;

        let wallet = ctx.wallet();
        let id = &req.swap_id;
        let refused = |e: &dyn std::fmt::Display| Refusal::Wallet(e.to_string());
        let our_pubkey = wallet.pubkey(id).map_err(|e| refused(&e))?;
        let status = Status::Funding {
            expires_at: started_at + config.funding_timeout.as_secs(),
            cancel_sent: false,
        };
        let (record, status) = match side {
            Side::Source => {
                let s = SwapIn::try_from(state).map_err(|e| Refusal::Malformed(e.to_string()))?;
                if s.chain == Chain::Liquid {
                    let ours = wallet.source_blinding_pubkey(id).map_err(|e| refused(&e))?;
                    blinding(s.user_blinding_pubkey, ours, s.service_blinding_pubkey)?;
                }
                if s.refund_pubkey != our_pubkey {
                    return Err(Refusal::OurPubkey.into());
                }
                let claim_invoice = req.claim_invoice.as_deref().ok_or(Refusal::ClaimInvoice)?;
                if s.claim_invoice != claim_invoice {
                    return Err(Refusal::ClaimInvoice.into());
                }
                if s.payment_hash != invoice::payment_hash(claim_invoice).map_err(invoice_error)? {
                    return Err(Refusal::PaymentHash.into());
                }
                let msat = invoice::amount_msat(claim_invoice).map_err(invoice_error)?;
                if Some(msat) != s.receive_amount.checked_mul(1000) {
                    return Err(Refusal::Invoice("amount is not the receive amount".into()).into());
                }
                let room = config.funding_timeout.as_secs()
                    + (u64::from(cc.min_confs) + 1) * cc.block_time;
                if invoice::expiry_at(claim_invoice).map_err(invoice_error)? < started_at + room {
                    return Err(Refusal::Invoice("expires too soon".into()).into());
                }
                csv_in_range(s.csv, 1, cc.max_refund_csv)?;
                let refund_address = req.user_refund_address.as_deref();
                if s.refund_address.as_deref() != refund_address {
                    return Err(Refusal::Address("differs from the one submitted".into()).into());
                }
                if let Some(refund_address) = refund_address {
                    validate_user_address(
                        "user_refund_address",
                        s.chain,
                        refund_address,
                        ctx.network(),
                    )
                    .map_err(|e| Refusal::Address(e.to_string()))?;
                }
                let record = SwapRecord {
                    version: SwapRecord::VERSION,
                    swap_id: s.swap_id,
                    payment_hash: s.payment_hash,
                    source: Endpoint::on_chain(s.terms(), refund_address.map(str::to_owned)),
                    dest: Endpoint::lightning(s.claim_invoice),
                    send_amount: s.send_amount,
                    receive_amount: s.receive_amount,
                    started_at,
                    status,
                    committed_at: None,
                    preimage: None,
                    sent: Sent::default(),
                };
                (record, ServerStatus::In(s.status))
            }
            Side::Dest => {
                let s = SwapOut::try_from(state).map_err(|e| Refusal::Malformed(e.to_string()))?;
                if s.chain == Chain::Liquid {
                    let ours = wallet.dest_blinding_pubkey(id).map_err(|e| refused(&e))?;
                    blinding(s.user_blinding_pubkey, ours, s.service_blinding_pubkey)?;
                }
                if s.claim_pubkey != our_pubkey {
                    return Err(Refusal::OurPubkey.into());
                }
                let claim_address = req
                    .user_claim_address
                    .as_deref()
                    .ok_or_else(|| Refusal::Address("none submitted".into()))?;
                if s.claim_address != claim_address {
                    return Err(Refusal::Address("differs from the one submitted".into()).into());
                }
                validate_user_address("user_claim_address", s.chain, claim_address, ctx.network())
                    .map_err(|e| Refusal::Address(e.to_string()))?;
                if s.payment_hash != wallet.payment_hash(id).map_err(|e| refused(&e))? {
                    return Err(Refusal::PaymentHash.into());
                }
                let inv = &s.claim_invoice;
                if invoice::payment_hash(inv).map_err(invoice_error)? != s.payment_hash {
                    return Err(Refusal::Invoice("payment hash is not ours".into()).into());
                }
                let msat = invoice::amount_msat(inv).map_err(invoice_error)?;
                if Some(msat) != s.send_amount.checked_mul(1000) {
                    return Err(Refusal::Invoice("amount is not the send amount".into()).into());
                }
                if invoice::network(inv).map_err(invoice_error)? != ctx.network() {
                    return Err(Refusal::Invoice("wrong network".into()).into());
                }
                if invoice::expiry_at(inv).map_err(invoice_error)? <= started_at {
                    return Err(Refusal::Invoice("expired".into()).into());
                }
                let cltv = invoice::min_final_cltv_expiry_delta(inv).map_err(invoice_error)?;
                let hold = (u64::from(s.csv) + u64::from(cc.max_htlc_hold)) * cc.block_time;
                if cltv.saturating_mul(LIGHTNING_BLOCK_TIME) > hold {
                    return Err(Refusal::Invoice("holds the payment past dest_csv".into()).into());
                }
                csv_in_range(s.csv, cc.min_confs + cc.claim_margin, 0xFFFF)?;
                let terms = s.terms();
                let record = SwapRecord {
                    version: SwapRecord::VERSION,
                    swap_id: s.swap_id,
                    payment_hash: s.payment_hash,
                    source: Endpoint::lightning(s.claim_invoice),
                    dest: Endpoint::on_chain(terms, Some(s.claim_address)),
                    send_amount: s.send_amount,
                    receive_amount: s.receive_amount,
                    started_at,
                    status,
                    committed_at: None,
                    preimage: None,
                    sent: Sent::default(),
                };
                (record, ServerStatus::Out(s.status))
            }
        };
        ctx.script(&record.terms(side)?)?;
        Ok((record, status))
    }
}

/// The checks both directions share, on the wire form so one function covers
/// both.
fn common(req: &CreateSwapRequest, side: Side, state: &SwapState) -> Result<(), Refusal> {
    if state.protocol_version != req.protocol_version {
        return Err(Refusal::ProtocolVersion {
            expected: req.protocol_version.clone(),
            found: state.protocol_version.clone(),
        });
    }
    if state.swap_id != req.swap_id {
        return Err(Refusal::SwapId(state.swap_id));
    }
    let (_, asked_chain, asked_asset) = asked(req)?;
    let (chain, asset) = match side {
        Side::Source => (state.source_chain, state.source_asset.as_deref()),
        Side::Dest => (state.dest_chain, state.dest_asset.as_deref()),
    };
    if chain != Some(asked_chain) {
        return Err(Refusal::Chain(asked_chain));
    }
    if asset != Some(asked_asset) {
        return Err(Refusal::Asset(asset.unwrap_or_default().to_owned()));
    }
    if state.receive_amount != req.receive_amount {
        return Err(Refusal::ReceiveAmount {
            expected: req.receive_amount,
            found: state.receive_amount,
        });
    }
    let fee = state.base_fee.checked_add(state.service_fee);
    let send_amount = fee.and_then(|fee| state.receive_amount.checked_add(fee));
    if send_amount != Some(state.send_amount) {
        return Err(Refusal::SendAmount {
            send_amount: state.send_amount,
            receive_amount: state.receive_amount,
            base_fee: state.base_fee,
            service_fee: state.service_fee,
        });
    }
    let fee = fee.unwrap_or(u64::MAX);
    if fee > req.swap_fee_limit {
        return Err(Refusal::FeeAboveLimit {
            fee,
            limit: req.swap_fee_limit,
        });
    }
    Ok(())
}

fn blinding(
    echoed: Option<PublicKey>,
    ours: PublicKey,
    servers: Option<PublicKey>,
) -> Result<(), Refusal> {
    match (echoed == Some(ours), servers.is_some()) {
        (true, true) => Ok(()),
        _ => Err(Refusal::BlindingPubkey),
    }
}

fn csv_in_range(csv: u32, min: u32, max: u32) -> Result<(), Refusal> {
    match (min..=max).contains(&csv) {
        true => Ok(()),
        false => Err(Refusal::Csv { csv, min, max }),
    }
}

fn invoice_error(e: invoice::InvoiceError) -> Refusal {
    Refusal::Invoice(e.to_string())
}

/// What `restore_all` found among the swaps the server lists and the store
/// lacks.
#[derive(Debug, Default)]
pub struct RestoreReport {
    pub restored: Vec<Uuid>,
    /// With the check each failed; `Unsigned` unless `trust_unsigned` is set.
    pub failed: Vec<(Uuid, DriverError)>,
}

/// Our own create request, out of the copy the server kept, once its
/// signature verifies against our identity key and its body names `swap_id`.
fn signed(
    swap_id: Uuid,
    body: &str,
    timestamp: i64,
    signature: &str,
    pubkey: &PublicKey,
) -> Result<(CreateSwapRequest, Timestamp), DriverError> {
    let signature = hex::decode(signature)
        .ok()
        .and_then(|bytes| Signature::from_slice(&bytes).ok())
        .ok_or(DriverError::BadSignature(swap_id))?;
    let digest = AuthCore::request_digest(
        &AuthRequest {
            method: "POST",
            target: "/v1/swap",
            body: body.as_bytes(),
            swap_id: &swap_id.to_string(),
            timestamp,
        },
        pubkey,
    );
    if !AuthCore::validate_signature(&digest, &signature, pubkey) {
        return Err(DriverError::BadSignature(swap_id));
    }
    let req: CreateSwapRequest =
        serde_json::from_str(body).map_err(|e| Refusal::Malformed(e.to_string()))?;
    if req.swap_id != swap_id {
        return Err(Refusal::SwapId(req.swap_id).into());
    }
    let started_at = u64::try_from(timestamp)
        .map_err(|_| Refusal::Malformed(format!("negative timestamp {timestamp}")))?;
    Ok((req, started_at))
}

/// The request the server's report implies, for a swap it holds no signed one
/// for. The terms are then its word, checked against each other and our keys.
fn reported(state: &SwapState, user_id: String) -> Result<CreateSwapRequest, Refusal> {
    let mut req = CreateSwapRequest {
        protocol_version: state.protocol_version.clone(),
        swap_id: state.swap_id,
        user_id,
        source_chain: None,
        source_asset_id: None,
        dest_chain: None,
        dest_asset_id: None,
        receive_amount: state.receive_amount,
        swap_fee_limit: state.swap_fee_limit,
        quote_id: None,
        claim_invoice: None,
        source_refund_pubkey: None,
        source_blinding_pubkey: None,
        dest_claim_pubkey: None,
        dest_blinding_pubkey: None,
        user_claim_address: None,
        user_refund_address: None,
        payment_hash: None,
        webhook: None,
    };
    match state.swap_type {
        SwapType::SwapIn => {
            req.source_chain = state.source_chain;
            req.source_asset_id = state.source_asset.clone();
            req.claim_invoice = state.claim_invoice.clone();
            req.source_refund_pubkey = state.source_refund_pubkey.clone();
            req.source_blinding_pubkey = state.source_user_blinding_pubkey.clone();
            req.user_refund_address = state.user_refund_address.clone();
        }
        SwapType::SwapOut => {
            req.dest_chain = state.dest_chain;
            req.dest_asset_id = state.dest_asset.clone();
            req.dest_claim_pubkey = state.dest_claim_pubkey.clone();
            req.dest_blinding_pubkey = state.dest_user_blinding_pubkey.clone();
            req.user_claim_address = state.user_claim_address.clone();
            req.payment_hash = Some(state.payment_hash.clone());
        }
        other => return Err(Refusal::SwapType(other)),
    }
    Ok(req)
}

impl<W: SwapWallet + MaybeSend + MaybeSync + 'static> Driver<W> {
    /// Rebuilds one swap from our own signed request, held by the server, or
    /// with `trust_unsigned` from its report when it holds none. A record
    /// already stored is returned as is.
    pub async fn restore(&self, swap_id: Uuid) -> Result<Swap<W>, DriverError> {
        if let Some(record) = self.core.store.load(&swap_id).await? {
            return Swap::new(self.clone(), record, None);
        }
        let state = self.core.client.get_swap(swap_id).await?;
        if state.swap_id != swap_id {
            return Err(Refusal::SwapId(state.swap_id).into());
        }
        self.restore_state(state).await
    }

    /// `restore` from a report already fetched with its `create_request_*`.
    async fn restore_state(&self, state: SwapState) -> Result<Swap<W>, DriverError> {
        let swap_id = state.swap_id;
        if let Some(record) = self.core.store.load(&swap_id).await? {
            return Swap::new(self.clone(), record, None);
        }
        let wallet = self.core.ctx.wallet();
        let (req, started_at) = match (
            state.create_request_body.as_deref(),
            state.create_request_timestamp,
            state.create_request_signature.as_deref(),
        ) {
            (Some(body), Some(timestamp), Some(signature)) => {
                let pubkey = wallet.identity_pubkey().map_err(wallet_error)?;
                signed(swap_id, body, timestamp, signature, &pubkey)?
            }
            _ if self.config.trust_unsigned => {
                let user_id = wallet.user_id().map_err(wallet_error)?;
                (reported(&state, user_id)?, state.created_at)
            }
            _ => return Err(DriverError::Unsigned(swap_id)),
        };
        let (mut record, status) = self.accept(&req, started_at, state.clone())?;

        match &status {
            ServerStatus::In(
                SwapInStatus::AwaitClaimTxConfirmation { .. } | SwapInStatus::Claimed { .. },
            ) => record.sent.refund_key = true,
            ServerStatus::Out(SwapOutStatus::Claimed { .. }) => record.sent.preimage = true,
            ServerStatus::In(SwapInStatus::Canceled { .. })
            | ServerStatus::Out(SwapOutStatus::Canceled { .. }) => record.sent.cancel = true,
            _ => {}
        }
        let terms = driven(&record)?;
        let server = poll(&terms, state);
        record.endpoint_mut(terms.side).server_privkey = server.key;
        record.preimage = server.preimage;

        self.core.store.save(&record).await?;
        Swap::new(self.clone(), record, Some(status))
    }

    /// Restores every swap the server lists under `query` that the store
    /// lacks. The filters are the caller's; `user_id`, `limit` and `offset`
    /// are the driver's.
    pub async fn restore_all(
        &self,
        mut query: SwapListQuery,
    ) -> Result<RestoreReport, DriverError> {
        let stored: HashSet<Uuid> = self
            .core
            .store
            .list()
            .await?
            .into_iter()
            .map(|record| record.swap_id)
            .collect();
        query.user_id = self.core.ctx.wallet().user_id().map_err(wallet_error)?;
        query.limit = MAX_SWAP_LIST_LIMIT;
        query.offset = 0;
        query.with_create_request = true;

        let mut report = RestoreReport::default();
        loop {
            let page = self.core.client.list_swaps(&query).await?;
            query.offset += page.swaps.len() as u32;
            let last = !page.pagination.has_next || page.swaps.is_empty();
            for state in page.swaps {
                let id = state.swap_id;
                if stored.contains(&id) {
                    continue;
                }
                match self.restore_state(state).await {
                    Ok(_) => report.restored.push(id),
                    Err(e) => report.failed.push((id, e)),
                }
            }
            if last {
                break;
            }
        }
        Ok(report)
    }
}
