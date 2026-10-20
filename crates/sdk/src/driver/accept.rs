//! Fixing the terms: a quote from `/info`, the acceptance checks a swap has to
//! pass before it is driven, and restoring one from our own signed request.

use std::collections::HashSet;

use anyswap_core::{
    PROTOCOL_VERSION,
    api::{CreateSwapRequest, InfoResponse, MAX_SWAP_LIST_LIMIT, SwapListQuery, SwapState},
    auth::{AuthCore, AuthRequest},
    fee, invoice,
    types::{
        Chain, NATIVE_ASSET, SwapInStatus as InStage, SwapNetwork, SwapOutStatus as OutStage,
        SwapType,
    },
    utils::validate_user_address,
};
use bitcoin::secp256k1::{PublicKey, schnorr::Signature};
use elements::AssetId;
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};
use uuid::Uuid;

use super::{
    Driver, Handle, MaybeSend, MaybeSync,
    decide::{deposit_deadline, funding_deadline},
    error::{DriverError, Refusal},
    observe::{driven, poll},
    record::{Endpoint, Sent, ServerStatus, Status, SwapRecord, Timestamp},
    rt, wallet_error,
};
use crate::{
    swap_in::SwapIn,
    swap_out::SwapOut,
    swap_script::Side,
    wallet::{SwapWallet, SwapWalletExt},
};

/// `GET /v1/info` reduced to two ends and one amount, with the `quote_id`
/// that locks it. An end is Lightning when its chain is `None`. Both amounts
/// are binding: the request carries them as they are here.
#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Quote {
    pub source_chain: Option<Chain>,
    pub source_asset_id: Option<String>,
    pub dest_chain: Option<Chain>,
    pub dest_asset_id: Option<String>,
    #[serde_as(as = "DisplayFromStr")]
    pub receive_amount: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub send_amount: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub base_fee: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub service_fee: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub fee: u64,
    pub quote_id: Uuid,
    pub expires_at: Timestamp,
    /// SwapIn: how long after creation the server waits for the deposit to
    /// confirm, in seconds, and so the expiry to give the claim invoice.
    #[serde(default)]
    pub await_opening_tx: Option<u64>,
}

/// Which end of a quote the caller pins: the amount they receive, or the
/// amount they spend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetAmount {
    Receive(u64),
    Send(u64),
}

pub(crate) fn quote(
    info: InfoResponse,
    network: SwapNetwork,
    policy_asset: Option<AssetId>,
    source: Option<Chain>,
    dest: Option<Chain>,
    target: TargetAmount,
    now: Timestamp,
) -> Result<Quote, DriverError> {
    if info.policy.network != network {
        return Err(Refusal::Network {
            expected: network,
            found: info.policy.network,
        }
        .into());
    }
    if info.policy.protocol_version != PROTOCOL_VERSION {
        return Err(Refusal::ProtocolVersion {
            expected: PROTOCOL_VERSION.into(),
            found: info.policy.protocol_version,
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
    let min_send = |amount| fee::min_send_for_receive(amount, service_ppm, base_fee);
    let out_of_range = || match target {
        TargetAmount::Receive(_) => DriverError::AmountOutOfRange { min, max },
        TargetAmount::Send(_) => DriverError::AmountOutOfRange {
            min: min_send(min).unwrap_or(u64::MAX),
            max: max
                .checked_add(1)
                .and_then(min_send)
                .map_or(u64::MAX, |next| next - 1),
        },
    };
    let (receive_amount, send_amount) = match target {
        TargetAmount::Receive(amount) => (amount, min_send(amount).ok_or_else(out_of_range)?),
        TargetAmount::Send(amount) => (
            fee::max_receive_for_send(amount, service_ppm, base_fee).ok_or_else(out_of_range)?,
            amount,
        ),
    };
    if receive_amount < min || receive_amount > max {
        return Err(out_of_range());
    }
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
        service_fee: send_amount - receive_amount - base_fee,
        fee: send_amount - receive_amount,
        quote_id,
        expires_at: now + info.quote_ttl,
        await_opening_tx: matches!(side, Side::Source).then_some(policy.timeouts.await_opening_tx),
    })
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
    ) -> Result<SwapRecord, DriverError> {
        let (side, chain, asset) = match (req.source_chain, req.dest_chain) {
            (Some(chain), None) => (Side::Source, chain, &req.source_asset_id),
            (None, Some(chain)) => (Side::Dest, chain, &req.dest_asset_id),
            _ => return Err(Refusal::Malformed("neither a SwapIn nor a SwapOut".into()).into()),
        };
        let asset = asset
            .as_deref()
            .ok_or_else(|| Refusal::Malformed("no asset".into()))?;
        let chain_config = &self.chain(chain)?.config;
        let (ctx, config) = (&self.core.ctx, &self.config);
        let swap_type = match side {
            Side::Source => SwapType::SwapIn,
            Side::Dest => SwapType::SwapOut,
        };
        if state.swap_type != swap_type {
            return Err(Refusal::SwapType(state.swap_type).into());
        }
        common(req, side, chain, asset, &state)?;

        let wallet = ctx.wallet();
        let id = &req.swap_id;
        let refused = |e: &dyn std::fmt::Display| Refusal::Wallet(e.to_string());
        let funding = |expires_at| Status::Funding {
            expires_at,
            cancel_sent: false,
        };
        let server_status = Some(state.status.clone());
        let record = match side {
            Side::Source => {
                let swap =
                    SwapIn::try_from(state).map_err(|e| Refusal::Malformed(e.to_string()))?;
                if swap.chain == Chain::Liquid {
                    let ours = wallet.source_blinding_pubkey(id).map_err(|e| refused(&e))?;
                    blinding(
                        swap.user_blinding_pubkey,
                        ours,
                        swap.service_blinding_pubkey,
                    )?;
                }
                if swap.refund_pubkey != wallet.source_refund_pubkey(id).map_err(|e| refused(&e))? {
                    return Err(Refusal::OurPubkey.into());
                }
                let claim_invoice = req.claim_invoice.as_deref().ok_or(Refusal::ClaimInvoice)?;
                if swap.claim_invoice != claim_invoice {
                    return Err(Refusal::ClaimInvoice.into());
                }
                let invoice_hash = invoice::payment_hash(claim_invoice).map_err(invoice_error)?;
                if swap.payment_hash != invoice_hash {
                    return Err(Refusal::PaymentHash.into());
                }
                let msat = invoice::amount_msat(claim_invoice).map_err(invoice_error)?;
                if Some(msat) != swap.receive_amount.checked_mul(1000) {
                    return Err(Refusal::Invoice("amount is not the receive amount".into()).into());
                }
                let room = (u64::from(chain_config.min_confs) + 1) * chain_config.block_time;
                if invoice::expiry_at(claim_invoice).map_err(invoice_error)? < started_at + room {
                    return Err(Refusal::Invoice("expires too soon".into()).into());
                }
                let expires_at =
                    deposit_deadline(started_at, Some(claim_invoice), config, chain_config);
                csv_in_range(swap.csv, 1, chain_config.max_refund_csv)?;
                let refund_address = req.user_refund_address.as_deref();
                if swap.refund_address.as_deref() != refund_address {
                    return Err(Refusal::Address("differs from the one submitted".into()).into());
                }
                if let Some(refund_address) = refund_address {
                    validate_user_address(
                        "user_refund_address",
                        swap.chain,
                        refund_address,
                        ctx.network(),
                    )
                    .map_err(|e| Refusal::Address(e.to_string()))?;
                }
                SwapRecord {
                    version: SwapRecord::VERSION,
                    swap_id: swap.swap_id,
                    payment_hash: swap.payment_hash,
                    source: Endpoint::on_chain(swap.terms(), refund_address.map(str::to_owned)),
                    dest: Endpoint::lightning(swap.claim_invoice),
                    send_amount: swap.send_amount,
                    receive_amount: swap.receive_amount,
                    base_fee: swap.base_fee,
                    service_fee: swap.service_fee,
                    started_at,
                    status: funding(expires_at),
                    funds: Vec::new(),
                    ended_at: None,
                    preimage: None,
                    sent: Sent::default(),
                    server_status,
                    server_cancel: None,
                }
            }
            Side::Dest => {
                let swap =
                    SwapOut::try_from(state).map_err(|e| Refusal::Malformed(e.to_string()))?;
                if swap.chain == Chain::Liquid {
                    let ours = wallet.dest_blinding_pubkey(id).map_err(|e| refused(&e))?;
                    blinding(
                        swap.user_blinding_pubkey,
                        ours,
                        swap.service_blinding_pubkey,
                    )?;
                }
                if swap.claim_pubkey != wallet.dest_claim_pubkey(id).map_err(|e| refused(&e))? {
                    return Err(Refusal::OurPubkey.into());
                }
                let claim_address = req
                    .user_claim_address
                    .as_deref()
                    .ok_or_else(|| Refusal::Address("none submitted".into()))?;
                if swap.claim_address != claim_address {
                    return Err(Refusal::Address("differs from the one submitted".into()).into());
                }
                let network = ctx.network();
                validate_user_address("user_claim_address", swap.chain, claim_address, network)
                    .map_err(|e| Refusal::Address(e.to_string()))?;
                if swap.payment_hash != wallet.payment_hash(id).map_err(|e| refused(&e))? {
                    return Err(Refusal::PaymentHash.into());
                }
                let hold_invoice = &swap.claim_invoice;
                let invoice_hash = invoice::payment_hash(hold_invoice).map_err(invoice_error)?;
                if invoice_hash != swap.payment_hash {
                    return Err(Refusal::Invoice("payment hash is not ours".into()).into());
                }
                let msat = invoice::amount_msat(hold_invoice).map_err(invoice_error)?;
                if Some(msat) != swap.send_amount.checked_mul(1000) {
                    return Err(Refusal::Invoice("amount is not the send amount".into()).into());
                }
                if invoice::network(hold_invoice).map_err(invoice_error)? != network {
                    return Err(Refusal::Invoice("wrong network".into()).into());
                }
                if invoice::expiry_at(hold_invoice).map_err(invoice_error)? <= started_at {
                    return Err(Refusal::Invoice("expired".into()).into());
                }
                let cltv =
                    invoice::min_final_cltv_expiry_delta(hold_invoice).map_err(invoice_error)?;
                if cltv > u64::from(config.max_htlc_hold) {
                    return Err(Refusal::Invoice("holds the payment too long".into()).into());
                }
                csv_in_range(
                    swap.csv,
                    chain_config.min_confs + chain_config.claim_margin,
                    0xFFFF,
                )?;
                let status = funding(funding_deadline(started_at, Some(hold_invoice), config));
                let terms = swap.terms();
                SwapRecord {
                    version: SwapRecord::VERSION,
                    swap_id: swap.swap_id,
                    payment_hash: swap.payment_hash,
                    source: Endpoint::lightning(swap.claim_invoice),
                    dest: Endpoint::on_chain(terms, Some(swap.claim_address)),
                    send_amount: swap.send_amount,
                    receive_amount: swap.receive_amount,
                    base_fee: swap.base_fee,
                    service_fee: swap.service_fee,
                    started_at,
                    status,
                    funds: Vec::new(),
                    ended_at: None,
                    preimage: None,
                    sent: Sent::default(),
                    server_status,
                    server_cancel: None,
                }
            }
        };
        ctx.script(&record.terms(side)?)?;
        Ok(record)
    }
}

/// The checks both directions share, on the wire form so one function covers
/// both. `asked_chain` and `asked_asset` are what the request asks for on
/// `side`.
fn common(
    req: &CreateSwapRequest,
    side: Side,
    asked_chain: Chain,
    asked_asset: &str,
    state: &SwapState,
) -> Result<(), Refusal> {
    if state.protocol_version != req.protocol_version {
        return Err(Refusal::ProtocolVersion {
            expected: req.protocol_version.clone(),
            found: state.protocol_version.clone(),
        });
    }
    if state.swap_id != req.swap_id {
        return Err(Refusal::SwapId(state.swap_id));
    }
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
    // Both amounts are ours by signature; the server's breakdown of the spread
    // between them only has to add up.
    let spread = state.base_fee.checked_add(state.service_fee);
    if state.send_amount != req.send_amount
        || spread != state.send_amount.checked_sub(state.receive_amount)
    {
        return Err(Refusal::SendAmount {
            expected: req.send_amount,
            found: state.send_amount,
            receive_amount: state.receive_amount,
            base_fee: state.base_fee,
            service_fee: state.service_fee,
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
    /// With the check each failed.
    pub failed: Vec<(Uuid, DriverError)>,
}

/// Our own create request, out of the copy the server kept in `state`, once
/// its signature verifies against our identity key and its body names the
/// swap.
fn signed(
    state: &SwapState,
    pubkey: &PublicKey,
) -> Result<(CreateSwapRequest, Timestamp), DriverError> {
    let swap_id = state.swap_id;
    let (Some(body), Some(timestamp), Some(signature)) = (
        state.create_request_body.as_deref(),
        state.create_request_timestamp,
        state.create_request_signature.as_deref(),
    ) else {
        return Err(DriverError::Unsigned(swap_id));
    };
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

impl<W: SwapWallet + MaybeSend + MaybeSync + 'static> Driver<W> {
    /// Rebuilds one swap from our own signed request, held by the server. It is
    /// `Restoring` until its first pass, which the loop makes, or `step`. A
    /// record already stored is returned as is.
    pub async fn restore(&self, swap_id: Uuid) -> Result<Handle<W>, DriverError> {
        if let Some(record) = self.core.store.load(&swap_id).await? {
            return Handle::new(self.clone(), &record);
        }
        let limit = self.config.server_timeout;
        let state = rt::bounded(limit, self.core.client.get_swap(swap_id)).await?;
        if state.swap_id != swap_id {
            return Err(Refusal::SwapId(state.swap_id).into());
        }
        self.restore_state(state).await
    }

    /// `restore` from a report already fetched with its `create_request_*`.
    async fn restore_state(&self, state: SwapState) -> Result<Handle<W>, DriverError> {
        let pubkey = self
            .core
            .ctx
            .wallet()
            .identity_pubkey()
            .map_err(wallet_error)?;
        let request = signed(&state, &pubkey)?;
        self.restore_signed(state, request).await
    }

    /// `restore` once the request in `state` verifies.
    async fn restore_signed(
        &self,
        state: SwapState,
        (req, started_at): (CreateSwapRequest, Timestamp),
    ) -> Result<Handle<W>, DriverError> {
        let swap_id = state.swap_id;
        let _guard = self.lock(swap_id).await;
        if let Some(record) = self.core.store.load(&swap_id).await? {
            return Handle::new(self.clone(), &record);
        }
        let mut record = self.accept(&req, started_at, state.clone())?;

        match record.server_status() {
            Some(ServerStatus::In(InStage::AwaitClaimTxConfirmation | InStage::Claimed)) => {
                record.sent.refund_key = true;
            }
            Some(ServerStatus::In(InStage::Canceled) | ServerStatus::Out(OutStage::Canceled)) => {
                record.sent.cancel = true;
            }
            _ => {}
        }
        let terms = driven(&record)?;
        // A swap that ended before the restore is watched from when it ended,
        // never from later than now.
        if record
            .server_status()
            .is_some_and(|status| status.is_terminal())
        {
            record.ended_at = Some(state.transitioned_at.min(anyswap_core::utils::now()));
        }
        let server = poll(&terms, state);
        record.endpoint_mut(terms.side).server_privkey = server.key;
        record.preimage = server.preimage;
        record.server_cancel = server.cancel;
        record.status = Status::Restoring;

        self.core.store.save(&record).await?;
        self.signal(swap_id);
        Handle::new(self.clone(), &record)
    }

    /// Restores every swap the server lists for this wallet that the store
    /// lacks, page by page. Paging goes on only while a page brings a swap
    /// not listed before whose request we signed, so a server cannot keep it
    /// going on repeated or made up entries.
    pub async fn restore_all(&self) -> Result<RestoreReport, DriverError> {
        let stored: HashSet<Uuid> = self.core.store.list().await?.into_iter().collect();
        let wallet = self.core.ctx.wallet();
        let pubkey = wallet.identity_pubkey().map_err(wallet_error)?;
        let mut query = SwapListQuery {
            user_id: wallet.user_id().map_err(wallet_error)?,
            limit: MAX_SWAP_LIST_LIMIT,
            offset: 0,
            with_create_request: true,
            ..SwapListQuery::default()
        };

        let mut report = RestoreReport::default();
        let mut seen = HashSet::new();
        let limit = self.config.server_timeout;
        loop {
            let page = rt::bounded(limit, self.core.client.list_swaps(&query)).await?;
            query.offset += page.swaps.len() as u32;
            let mut ours = false;
            for state in page.swaps {
                let id = state.swap_id;
                if !seen.insert(id) {
                    continue;
                }
                if stored.contains(&id) {
                    ours = true;
                    continue;
                }
                let restored = match signed(&state, &pubkey) {
                    Ok(request) => {
                        ours = true;
                        self.restore_signed(state, request).await
                    }
                    Err(e) => Err(e),
                };
                match restored {
                    Ok(_) => report.restored.push(id),
                    Err(e) => report.failed.push((id, e)),
                }
            }
            if !page.pagination.has_next || !ours {
                break;
            }
        }
        Ok(report)
    }
}
