//! The loop: `observe` reads the chain and the server into something
//! verified, `decide` is the pure choice of what to do, and each tick applies
//! it and saves what changed.

use std::collections::BTreeMap;

use anyswap_core::{
    api::{CancelRequest, RevealClaimRequest, RevealPreimageRequest, SwapState},
    secp,
    types::SwapType,
    utils::sha256_payment_hash,
};
use bitcoin::secp256k1::SecretKey;

use super::{
    Driver, MaybeSend, MaybeSync, Registered,
    config::{ChainConfig, Config},
    error::DriverError,
    record::{Height, Status, SwapRecord, Timestamp},
    server::ServerClient,
    wallet_error,
};
use crate::{
    swap_in::{SwapIn, SwapInStatus},
    swap_out::{SwapOut, SwapOutStatus},
    swap_script::{Side, SwapScript, Terms, Tx},
    utxo::SwapUtxo,
    wallet::SwapWallet,
};

/// One pass's view of the world, verified; nothing in it is left to believe.
#[derive(Clone, Debug)]
pub struct Observed {
    pub now: Timestamp,
    pub tip: Height,
    /// The chain's estimate at `fee_conf_target`, in sat/vB.
    pub fee_rate: f64,
    /// Every output ever under the script that is this swap's, oldest first.
    pub outputs: Vec<Output>,
    /// One poll of the server, `None` when it did not answer.
    pub server: Option<Server>,
}

#[derive(Clone, Debug)]
pub struct Output {
    pub utxo: SwapUtxo,
    pub value: u64,
    pub height: Option<Height>,
    pub spend: Option<Spender>,
}

/// The transaction spending an output.
#[derive(Clone, Debug)]
pub struct Spender {
    pub txid: String,
    pub tx_hex: String,
    pub height: Option<Height>,
    /// A witness item hashing to `payment_hash`.
    pub preimage: Option<[u8; 32]>,
    /// Pays the end's `destination`.
    pub pays_address: bool,
}

/// One successful poll of the server, each part taken on its own terms, so a
/// status we cannot parse costs the label and nothing else.
#[derive(Clone, Debug)]
pub struct Server {
    /// The report as parsed; `None` when it spoke a status we do not know.
    pub status: Option<ServerStatus>,
    /// Its key, verified against its pubkey in the terms.
    pub key: Option<SecretKey>,
    /// Its preimage, verified against `payment_hash`.
    pub preimage: Option<[u8; 32]>,
}

#[derive(Clone, Debug)]
pub enum ServerStatus {
    In(SwapInStatus),
    Out(SwapOutStatus),
}

impl Observed {
    pub fn confs(&self, height: Option<Height>) -> u32 {
        height.map_or(0, |h| self.tip.saturating_sub(h) + 1)
    }

    /// The one output paying exactly `amount`; two mean none at all.
    pub fn unique(&self, amount: u64) -> Option<&Output> {
        let mut found = self.outputs.iter().filter(|o| o.value == amount);
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }

    pub fn server_privkey(&self) -> Option<SecretKey> {
        self.server.as_ref()?.key
    }

    pub fn server_canceled(&self) -> bool {
        matches!(
            self.server.as_ref().and_then(|s| s.status.as_ref()),
            Some(ServerStatus::In(SwapInStatus::Canceled { .. }))
                | Some(ServerStatus::Out(SwapOutStatus::Canceled { .. }))
        )
    }

    /// A verified preimage from either source.
    pub fn preimage(&self) -> Option<[u8; 32]> {
        self.server
            .as_ref()
            .and_then(|s| s.preimage)
            .or_else(|| self.outputs.iter().find_map(|o| o.spend.as_ref()?.preimage))
    }
}

/// The terms of the one on-chain end the loop drives.
pub(crate) fn driven(record: &SwapRecord) -> Result<Terms, DriverError> {
    match record.swap_type()? {
        SwapType::SwapIn => record.terms(Side::Source),
        SwapType::SwapOut => record.terms(Side::Dest),
        other => Err(DriverError::Unsupported(other)),
    }
}

pub(crate) async fn observe(
    record: &SwapRecord,
    terms: &Terms,
    script: &SwapScript,
    chain: &Registered,
    client: &dyn ServerClient,
    fee_conf_target: u32,
) -> Result<Observed, DriverError> {
    let tip = chain.client.height().await?;
    let fee_rate = chain.client.fee_rate(fee_conf_target).await?;
    let history = chain.client.address_history(&script.address()).await?;
    let txs = history
        .into_iter()
        .map(|h| Tx::decode(terms.chain, &h.hex).map(|tx| (h, tx)))
        .collect::<Result<Vec<_>, _>>()?;

    let mut outputs: Vec<Output> = txs
        .iter()
        .flat_map(|(h, tx)| {
            script.outputs(tx).into_iter().map(|utxo| Output {
                value: script.value(&utxo).unwrap_or_default(),
                utxo,
                height: h.height,
                spend: None,
            })
        })
        .collect();
    let payout = record
        .endpoint(terms.side)
        .destination
        .as_deref()
        .map(|a| script.destination_script(a))
        .transpose()?;
    for (h, tx) in &txs {
        for (vin, prev) in tx.inputs().into_iter().enumerate() {
            if let Some(output) = outputs.iter_mut().find(|o| o.utxo.outpoint() == prev) {
                output.spend = Some(Spender {
                    txid: tx.txid(),
                    tx_hex: h.hex.clone(),
                    height: h.height,
                    preimage: tx.preimage_at(vin as u32, &terms.payment_hash),
                    pays_address: payout.as_deref().is_some_and(|p| tx.pays(p)),
                });
            }
        }
    }
    outputs.sort_by(|a, b| {
        let key = |o: &Output| {
            (
                o.height.unwrap_or(Height::MAX),
                o.utxo.txid.clone(),
                o.utxo.vout,
            )
        };
        key(a).cmp(&key(b))
    });

    let server = client
        .get_swap(record.swap_id)
        .await
        .ok()
        .map(|state| poll(terms, state));

    Ok(Observed {
        now: anyswap_core::utils::now(),
        tip,
        fee_rate,
        outputs,
        server,
    })
}

/// Splits one response into the three things ever taken from one, each on its
/// own terms: the key against the script's pubkey, the preimage against
/// `payment_hash` (ours on a SwapOut, so not taken), the status as parsed.
pub(crate) fn poll(terms: &Terms, state: SwapState) -> Server {
    let key = match terms.side {
        Side::Source => state.source_claim_privkey.as_deref(),
        Side::Dest => state.dest_refund_privkey.as_deref(),
    }
    .and_then(|hex| crate::codec::parse_privkey(hex).ok())
    .filter(|key| key.public_key(secp()) == terms.server_pubkey);

    let preimage = match terms.side {
        Side::Source => state.preimage.as_deref(),
        Side::Dest => None,
    }
    .and_then(|hex| crate::codec::parse_hash32(hex).ok())
    .filter(|preimage| sha256_payment_hash(preimage) == terms.payment_hash);

    let status = match terms.side {
        Side::Source => SwapIn::try_from(state)
            .ok()
            .map(|s| ServerStatus::In(s.status)),
        Side::Dest => SwapOut::try_from(state)
            .ok()
            .map(|s| ServerStatus::Out(s.status)),
    };

    Server {
        status,
        key,
        preimage,
    }
}

/// Something that changes the world. Everything else a pass does is reading.
#[derive(Clone, Debug)]
pub enum Action {
    Send(Message),
    Spend(Spend),
    /// Record that the preimage is about to go out, and nothing else.
    Commit,
    /// The status the swap ends on.
    Close(Status),
}

/// The three things we ever tell the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    Cancel,
    RefundKey,
    Preimage,
}

#[derive(Clone, Debug)]
pub enum Spend {
    Claim {
        utxos: Vec<SwapUtxo>,
        path: Path,
        fee_rate: f64,
    },
    Rebroadcast(String),
}

#[derive(Clone, Debug)]
pub enum Path {
    Keypath { server_privkey: SecretKey },
    Preimage,
}

/// What a pass does and the status it resolves, assuming the action succeeds.
/// A `Spend` resolves to `Spending` with the txid the broadcast returns.
#[derive(Clone, Debug)]
pub struct Decision {
    pub action: Option<Action>,
    pub status: Status,
}

fn hold(status: Status) -> Decision {
    Decision {
        action: None,
        status,
    }
}

fn act(action: Action, status: Status) -> Decision {
    Decision {
        action: Some(action),
        status,
    }
}

fn close(status: Status) -> Decision {
    act(Action::Close(status.clone()), status)
}

/// `terms` are the on-chain end's, the one the loop drives.
pub fn decide(
    r: &SwapRecord,
    terms: &Terms,
    o: &Observed,
    config: &Config,
    cc: &ChainConfig,
) -> Decision {
    match terms.side {
        Side::Source => swap_in(r, terms, o, config, cc),
        Side::Dest => swap_out(r, terms, o, config, cc),
    }
}

fn swap_in(
    r: &SwapRecord,
    terms: &Terms,
    o: &Observed,
    config: &Config,
    cc: &ChainConfig,
) -> Decision {
    let keypath = r.source.server_privkey.is_some() || o.server_privkey().is_some();
    match &r.status {
        Status::Completed { .. } => return hold(r.status.clone()),
        Status::Refund { expires_at, .. } => {
            return hold(refund(terms, o, *expires_at, keypath));
        }
        _ => {}
    }
    let deposit = o.unique(r.send_amount);
    let preimage = r.preimage.or_else(|| o.preimage());
    if let (Some(_), Some(deposit)) = (preimage, deposit) {
        if !r.sent.refund_key {
            return act(
                Action::Send(Message::RefundKey),
                confirming(terms, o, cc, deposit),
            );
        }
        return close(Status::Completed {
            opening: deposit.utxo.txid.clone(),
            payout: deposit.spend.as_ref().map(|s| s.txid.clone()),
        });
    }
    let matured = deposit.is_some_and(|d| o.confs(d.height) >= terms.csv);
    if keypath || matured {
        let expires_at = o.now + config.refund_timeout.as_secs();
        return close(refund(terms, o, expires_at, keypath));
    }
    let Some(deposit) = deposit else {
        let expires_at = funding_deadline(r, config);
        let cancel = o.now >= expires_at && !r.sent.cancel;
        return Decision {
            action: cancel.then_some(Action::Send(Message::Cancel)),
            status: Status::Funding {
                expires_at,
                cancel_sent: r.sent.cancel || cancel,
            },
        };
    };
    hold(confirming(terms, o, cc, deposit))
}

fn swap_out(
    r: &SwapRecord,
    terms: &Terms,
    o: &Observed,
    config: &Config,
    cc: &ChainConfig,
) -> Decision {
    if r.status.is_terminal() {
        return hold(r.status.clone());
    }
    let server_privkey = r.dest.server_privkey.or(o.server_privkey());
    let fee_rate = o.fee_rate.min(cc.max_fee_rate);
    let committed = r.committed_at.is_some();

    let Some(opening) = o.unique(r.receive_amount) else {
        if o.server_canceled() {
            return close(Status::Canceled { opening: None });
        }
        let expires_at = funding_deadline(r, config);
        let cancel = !committed && o.now >= expires_at && !r.sent.cancel;
        return Decision {
            action: cancel.then_some(Action::Send(Message::Cancel)),
            status: Status::Funding {
                expires_at,
                cancel_sent: r.sent.cancel || cancel,
            },
        };
    };
    let txid = opening.utxo.txid.clone();
    let confs = o.confs(opening.height);
    let matures_at = opening.height.unwrap_or(o.tip) + terms.csv;

    match &opening.spend {
        Some(spend) if spend.pays_address => {
            let spend_confs = o.confs(spend.height);
            if spend_confs >= cc.min_confs {
                return close(Status::Completed {
                    opening: txid,
                    payout: Some(spend.txid.clone()),
                });
            }
            let status = Status::Spending {
                txid: spend.txid.clone(),
                confs: spend_confs,
            };
            match spend.height {
                None => act(
                    Action::Spend(Spend::Rebroadcast(spend.tx_hex.clone())),
                    status,
                ),
                Some(_) => hold(status),
            }
        }
        Some(spend) => {
            let spend_confs = o.confs(spend.height);
            if spend_confs >= cc.min_confs {
                return close(match committed {
                    true => Status::Lost { opening: txid },
                    false => Status::Canceled {
                        opening: Some(txid),
                    },
                });
            }
            hold(match committed {
                true => Status::Spending {
                    txid: spend.txid.clone(),
                    confs: spend_confs,
                },
                false => confirming(terms, o, cc, opening),
            })
        }
        None => {
            let claim = |path| {
                Action::Spend(Spend::Claim {
                    utxos: vec![opening.utxo.clone()],
                    path,
                    fee_rate,
                })
            };
            if let Some(server_privkey) = server_privkey {
                let status = Status::CoopKey {
                    expires_at: o.now,
                    matures_at,
                };
                return act(claim(Path::Keypath { server_privkey }), status);
            }
            if let Some(committed_at) = r.committed_at {
                let expires_at = committed_at + config.coop_timeout.as_secs();
                let status = Status::CoopKey {
                    expires_at,
                    matures_at,
                };
                return if o.now >= expires_at {
                    act(claim(Path::Preimage), status)
                } else if !r.sent.preimage {
                    act(Action::Send(Message::Preimage), status)
                } else {
                    hold(status)
                };
            }
            let deep = opening.height.is_some() && confs >= cc.min_confs;
            if deep && terms.csv.saturating_sub(confs) >= cc.claim_margin {
                let status = Status::CoopKey {
                    expires_at: o.now + config.coop_timeout.as_secs(),
                    matures_at,
                };
                return act(Action::Commit, status);
            }
            hold(confirming(terms, o, cc, opening))
        }
    }
}

fn funding_deadline(r: &SwapRecord, config: &Config) -> Timestamp {
    r.started_at + config.funding_timeout.as_secs()
}

fn confirming(terms: &Terms, o: &Observed, cc: &ChainConfig, opening: &Output) -> Status {
    Status::Confirming {
        txid: opening.utxo.txid.clone(),
        confs: o.confs(opening.height),
        needed: cc.min_confs,
        matures_at: opening.height.map(|h| h + terms.csv),
    }
}

/// `Refund` as the chain shows it: everything under the script is ours.
fn refund(terms: &Terms, o: &Observed, expires_at: Timestamp, keypath: bool) -> Status {
    let unspent = o.outputs.iter().filter(|x| x.spend.is_none());
    let refundable = unspent.clone().map(|x| x.value).sum();
    let matures_at = match keypath {
        true => None,
        false => unspent
            .filter_map(|x| x.height.map(|h| h + terms.csv))
            .min(),
    };
    let refunded: BTreeMap<&str, bool> = o
        .outputs
        .iter()
        .filter_map(|x| x.spend.as_ref())
        .map(|s| (s.txid.as_str(), s.height.is_some()))
        .collect();
    Status::Refund {
        expires_at,
        refundable,
        matures_at,
        refunded: refunded
            .into_iter()
            .map(|(txid, confirmed)| (txid.to_owned(), confirmed))
            .collect(),
    }
}

/// The outputs that are ours to take: everything under the script once the
/// swap is over, only the strays while the deposit is in play or was claimed.
pub(crate) fn ours<'a>(r: &SwapRecord, o: &'a Observed) -> impl Iterator<Item = &'a Output> {
    let deposit = match r.status {
        Status::Refund { .. } => None,
        _ => o.unique(r.send_amount).map(|d| d.utxo.outpoint()),
    };
    o.outputs
        .iter()
        .filter(move |x| Some(x.utxo.outpoint()) != deposit)
}

/// The outputs a refund can spend now: unspent, and past the CSV unless the
/// server's key takes the keypath.
pub(crate) fn spendable(
    r: &SwapRecord,
    terms: &Terms,
    o: &Observed,
    keypath: bool,
) -> Vec<SwapUtxo> {
    ours(r, o)
        .filter(|x| x.spend.is_none())
        .filter(|x| keypath || x.height.is_some_and(|h| o.tip + 1 >= h + terms.csv))
        .map(|x| x.utxo.clone())
        .collect()
}

impl<W: SwapWallet + MaybeSend + MaybeSync + 'static> Driver<W> {
    /// One tick: observe, decide, apply, save and announce what changed, and
    /// take a refund where a wallet stands behind the driver. Returns the
    /// server's report for the handle to show.
    pub(crate) async fn tick(
        &self,
        record: &mut SwapRecord,
    ) -> Result<Option<ServerStatus>, DriverError> {
        let terms = driven(record)?;
        let chain = self.chain(terms.chain)?;
        let script = self.core.ctx.script(&terms)?;
        let observed = observe(
            record,
            &terms,
            &script,
            chain,
            &self.core.client,
            self.config.fee_conf_target,
        )
        .await?;
        let before = record.clone();

        if let Some(key) = observed.server_privkey() {
            record.endpoint_mut(terms.side).server_privkey = Some(key);
        }
        if let Some(preimage) = observed.preimage() {
            record.preimage = Some(preimage);
        }
        let decision = decide(record, &terms, &observed, &self.config, &chain.config);
        self.apply(record, &terms, &script, &observed, decision, chain)
            .await?;
        let changed = *record != before;
        if changed {
            self.core.store.save(record).await?;
        }

        let end = record.endpoint(terms.side);
        if let Status::Refund { .. } = record.status
            && !spendable(record, &terms, &observed, end.server_privkey.is_some()).is_empty()
            && let Some(destination) = end.destination.clone().or_else(|| {
                self.destinations
                    .as_ref()
                    .map(|d| d.refund_address(terms.chain))
            })
        {
            self.refund_from(record, &terms, &script, &observed, chain, &destination)
                .await?;
        }
        self.core.errors.lock().unwrap().remove(&record.swap_id);
        let server = observed.server.and_then(|s| s.status);
        if changed {
            self.emit(record.clone(), server.clone());
        }
        Ok(server)
    }

    /// Takes what the script holds for us to `destination`, or nothing.
    pub(crate) async fn refund(
        &self,
        record: &SwapRecord,
        destination: &str,
    ) -> Result<Option<String>, DriverError> {
        let terms = driven(record)?;
        let chain = self.chain(terms.chain)?;
        let script = self.core.ctx.script(&terms)?;
        let observed = observe(
            record,
            &terms,
            &script,
            chain,
            &self.core.client,
            self.config.fee_conf_target,
        )
        .await?;
        self.refund_from(record, &terms, &script, &observed, chain, destination)
            .await
    }

    async fn apply(
        &self,
        record: &mut SwapRecord,
        terms: &Terms,
        script: &SwapScript,
        observed: &Observed,
        decision: Decision,
        chain: &Registered,
    ) -> Result<(), DriverError> {
        let Decision { action, status } = decision;
        record.status = match action {
            None => status,
            Some(Action::Close(closed)) => closed,
            Some(Action::Commit) => {
                record.committed_at = Some(observed.now);
                status
            }
            Some(Action::Send(message)) => {
                self.send(record, message, "deadline passed").await?;
                status
            }
            Some(Action::Spend(spend)) => {
                let txid = self.spend(record, terms, script, spend, chain).await?;
                record.committed_at.get_or_insert(observed.now);
                Status::Spending { txid, confs: 0 }
            }
        };
        Ok(())
    }

    async fn send(
        &self,
        record: &mut SwapRecord,
        message: Message,
        reason: &str,
    ) -> Result<(), DriverError> {
        let id = record.swap_id;
        let client = &self.core.client;
        let wallet = self.core.ctx.wallet();
        let result = match message {
            Message::Cancel => {
                let req = CancelRequest {
                    cancel_message: reason.into(),
                };
                client.cancel(id, &req).await
            }
            Message::RefundKey => {
                let key = wallet.privkey(&id).map_err(wallet_error)?;
                let req = RevealClaimRequest {
                    source_refund_privkey: hex::encode(key.secret_bytes()),
                };
                client.reveal_claim(id, &req).await
            }
            Message::Preimage => {
                let preimage = wallet.preimage(&id).map_err(wallet_error)?;
                let req = RevealPreimageRequest {
                    preimage: hex::encode(preimage),
                };
                client.reveal_preimage(id, &req).await
            }
        };
        match result {
            Ok(()) => {}
            Err(e) if e.transition_rejected() => {}
            Err(e) => return Err(e.into()),
        }
        *match message {
            Message::Cancel => &mut record.sent.cancel,
            Message::RefundKey => &mut record.sent.refund_key,
            Message::Preimage => &mut record.sent.preimage,
        } = true;
        Ok(())
    }

    async fn spend(
        &self,
        record: &SwapRecord,
        terms: &Terms,
        script: &SwapScript,
        spend: Spend,
        chain: &Registered,
    ) -> Result<String, DriverError> {
        let tx_hex = match spend {
            Spend::Rebroadcast(tx_hex) => tx_hex,
            Spend::Claim {
                utxos,
                path,
                fee_rate,
            } => {
                let id = record.swap_id;
                let wallet = self.core.ctx.wallet();
                let ours = wallet.privkey(&id).map_err(wallet_error)?;
                let end = record.endpoint(terms.side);
                let destination = end.destination.as_deref().unwrap_or_default();
                match path {
                    Path::Keypath { server_privkey } => {
                        script.keypath(&utxos, ours, server_privkey, destination, fee_rate)?
                    }
                    Path::Preimage => {
                        let preimage = wallet.preimage(&id).map_err(wallet_error)?;
                        script.claim_by_preimage(&utxos, ours, preimage, destination, fee_rate)?
                    }
                }
            }
        };
        Ok(chain.client.broadcast(&tx_hex).await?)
    }

    async fn refund_from(
        &self,
        record: &SwapRecord,
        terms: &Terms,
        script: &SwapScript,
        observed: &Observed,
        chain: &Registered,
        destination: &str,
    ) -> Result<Option<String>, DriverError> {
        let end = record.endpoint(terms.side);
        let server_privkey = end.server_privkey.or(observed.server_privkey());
        let fee_rate = observed.fee_rate.min(chain.config.max_fee_rate);
        let utxos = spendable(record, terms, observed, server_privkey.is_some());
        let (utxos, _) = script.economical(&utxos, fee_rate)?;
        if utxos.is_empty() {
            return Ok(None);
        }
        let ours = self
            .core
            .ctx
            .wallet()
            .privkey(&record.swap_id)
            .map_err(wallet_error)?;
        let tx_hex = match server_privkey {
            Some(server) => script.keypath(&utxos, server, ours, destination, fee_rate)?,
            None => script.refund_by_csv(&utxos, ours, destination, fee_rate)?,
        };
        Ok(Some(chain.client.broadcast(&tx_hex).await?))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use anyswap_core::{secp, types::Chain};
    use bitcoin::secp256k1::SecretKey;
    use uuid::Uuid;

    use super::*;
    use crate::{
        driver::record::{Endpoint, Sent},
        swap_in::{ErrorType, SwapInStatus},
        swap_out::SwapOutStatus,
        swap_script::Terms,
    };

    const SERVER_KEY: [u8; 32] = [21; 32];

    fn server_key() -> SecretKey {
        SecretKey::from_slice(&SERVER_KEY).unwrap()
    }

    fn record(side: Side) -> SwapRecord {
        let terms = Terms {
            swap_id: Uuid::from_u128(1),
            side,
            chain: Chain::Bitcoin,
            asset: "native".into(),
            csv: 144,
            payment_hash: [7; 32],
            server_pubkey: server_key().public_key(secp()),
            server_blinding_pubkey: None,
        };
        let lightning = Endpoint::lightning("lnbcrt1".into());
        let (source, dest) = match side {
            Side::Source => (Endpoint::on_chain(terms, None), lightning),
            Side::Dest => (lightning, Endpoint::on_chain(terms, Some("bcrt1q".into()))),
        };
        SwapRecord {
            version: SwapRecord::VERSION,
            swap_id: Uuid::from_u128(1),
            payment_hash: [7; 32],
            source,
            dest,
            send_amount: 101_000,
            receive_amount: 100_000,
            started_at: 1_000,
            status: Status::Funding {
                expires_at: 2_200,
                cancel_sent: false,
            },
            committed_at: None,
            preimage: None,
            sent: Sent::default(),
        }
    }

    fn terms(r: &SwapRecord) -> Terms {
        driven(r).unwrap()
    }

    fn with_key(mut r: SwapRecord) -> SwapRecord {
        r.endpoint_mut(terms(&r).side).server_privkey = Some(server_key());
        r
    }

    fn committed(mut r: SwapRecord) -> SwapRecord {
        r.committed_at = Some(1_400);
        r
    }

    fn observed(tip: u32, outputs: Vec<Output>) -> Observed {
        Observed {
            now: 1_500,
            tip,
            fee_rate: 2.0,
            outputs,
            server: None,
        }
    }

    fn output(txid: &str, value: u64, height: Option<u32>) -> Output {
        Output {
            utxo: SwapUtxo::bitcoin(txid, 0, value),
            value,
            height,
            spend: None,
        }
    }

    fn spender(txid: &str, height: Option<u32>) -> Spender {
        Spender {
            txid: txid.into(),
            tx_hex: "00".into(),
            height,
            preimage: None,
            pays_address: false,
        }
    }

    fn config() -> Config {
        Config {
            funding_timeout: Duration::from_secs(1_200),
            coop_timeout: Duration::from_secs(300),
            refund_timeout: Duration::from_secs(3_600),
            ..Default::default()
        }
    }

    fn chain_config() -> ChainConfig {
        ChainConfig {
            claim_margin: 100,
            ..ChainConfig::bitcoin()
        }
    }

    fn decide_in(r: &SwapRecord, o: &Observed) -> Decision {
        decide(r, &terms(r), o, &config(), &chain_config())
    }

    fn canceled_report(privkey: SecretKey, key: Option<SecretKey>) -> Server {
        Server {
            status: Some(ServerStatus::In(SwapInStatus::Canceled {
                error_code: ErrorType::UserCanceled,
                claim_privkey: privkey,
                cancel_message: String::new(),
            })),
            key,
            preimage: None,
        }
    }

    #[test]
    fn swap_in_waits_for_a_deposit() {
        let d = decide_in(&record(Side::Source), &observed(100, vec![]));
        assert!(d.action.is_none());
        assert!(matches!(
            d.status,
            Status::Funding {
                expires_at: 2_200,
                cancel_sent: false
            }
        ));
    }

    #[test]
    fn swap_in_cancels_once_past_the_funding_timeout() {
        let r = record(Side::Source);
        let mut o = observed(100, vec![]);
        o.now = 2_200;
        let d = decide_in(&r, &o);
        assert!(matches!(d.action, Some(Action::Send(Message::Cancel))));
        assert!(matches!(
            d.status,
            Status::Funding {
                cancel_sent: true,
                ..
            }
        ));

        let sent = SwapRecord {
            sent: Sent {
                cancel: true,
                ..Sent::default()
            },
            ..r
        };
        assert!(decide_in(&sent, &o).action.is_none());
    }

    #[test]
    fn swap_in_holds_the_cancel_once_the_deposit_is_seen() {
        let r = record(Side::Source);
        let mut o = observed(100, vec![output("a", 101_000, None)]);
        o.now = 5_000;
        let d = decide_in(&r, &o);
        assert!(d.action.is_none());
        assert!(matches!(d.status, Status::Confirming { confs: 0, .. }));
    }

    #[test]
    fn swap_in_two_exact_deposits_are_no_deposit() {
        let o = observed(
            100,
            vec![
                output("a", 101_000, Some(100)),
                output("b", 101_000, Some(100)),
            ],
        );
        assert!(matches!(
            decide_in(&record(Side::Source), &o).status,
            Status::Funding { .. }
        ));
    }

    #[test]
    fn swap_in_counts_confirmations_on_the_deposit() {
        let o = observed(102, vec![output("a", 101_000, Some(100))]);
        let d = decide_in(&record(Side::Source), &o);
        assert!(d.action.is_none());
        assert!(matches!(
            d.status,
            Status::Confirming {
                confs: 3,
                needed: 3,
                matures_at: Some(244),
                ..
            }
        ));
    }

    #[test]
    fn swap_in_reveals_the_refund_key_then_completes_on_the_preimage() {
        let r = record(Side::Source);
        let mut deposit = output("a", 101_000, Some(100));
        deposit.spend = Some(Spender {
            preimage: Some([1; 32]),
            ..spender("claim", Some(101))
        });
        let o = observed(102, vec![deposit]);

        let d = decide_in(&r, &o);
        assert!(matches!(d.action, Some(Action::Send(Message::RefundKey))));
        assert!(matches!(d.status, Status::Confirming { .. }));

        let sent = SwapRecord {
            sent: Sent {
                refund_key: true,
                ..Sent::default()
            },
            ..r
        };
        let d = decide_in(&sent, &o);
        assert!(matches!(d.action, Some(Action::Close(_))));
        assert!(matches!(
            d.status,
            Status::Completed { ref opening, payout: Some(ref payout) } if opening == "a" && payout == "claim"
        ));
    }

    #[test]
    fn swap_in_completes_from_the_banked_preimage_after_a_keypath_claim() {
        let r = SwapRecord {
            preimage: Some([1; 32]),
            sent: Sent {
                refund_key: true,
                ..Sent::default()
            },
            ..record(Side::Source)
        };
        let mut deposit = output("a", 101_000, Some(100));
        deposit.spend = Some(spender("claim", Some(103)));
        let d = decide_in(&r, &observed(103, vec![deposit]));
        assert!(matches!(
            d.status,
            Status::Completed { payout: Some(ref payout), .. } if payout == "claim"
        ));
    }

    #[test]
    fn poll_takes_key_and_preimage_from_an_unknown_status() {
        let preimage = [1u8; 32];
        let mut r = record(Side::Source);
        r.payment_hash = sha256_payment_hash(&preimage);
        let state: SwapState = serde_json::from_value(serde_json::json!({
            "protocol_version": "1",
            "swap_id": r.swap_id,
            "swap_type": "SwapIn",
            "status": "Surprise",
            "receive_amount": 0,
            "send_amount": 0,
            "base_fee": 0,
            "service_fee": 0,
            "swap_fee_limit": 0,
            "payment_hash": hex::encode(r.payment_hash),
            "transitioned_at": 0,
            "created_at": 0,
            "preimage": hex::encode(preimage),
            "source_claim_privkey": hex::encode(server_key().secret_bytes()),
        }))
        .unwrap();

        let server = poll(&terms(&r), state);
        assert!(server.status.is_none());
        assert_eq!(server.key, Some(server_key()));
        assert_eq!(server.preimage, Some(preimage));
    }

    #[test]
    fn swap_in_ignores_a_key_that_is_not_the_servers() {
        let mut o = observed(100, vec![]);
        o.server = Some(canceled_report(
            SecretKey::from_slice(&[3; 32]).unwrap(),
            None,
        ));
        assert!(matches!(
            decide_in(&record(Side::Source), &o).status,
            Status::Funding { .. }
        ));
    }

    #[test]
    fn swap_in_banks_the_key_without_a_parseable_report() {
        let mut o = observed(100, vec![output("a", 101_000, None)]);
        o.server = Some(Server {
            status: None,
            key: Some(server_key()),
            preimage: None,
        });
        assert!(matches!(
            decide_in(&record(Side::Source), &o).status,
            Status::Refund {
                refundable: 101_000,
                matures_at: None,
                ..
            }
        ));
    }

    #[test]
    fn swap_in_closes_to_refund_on_the_servers_key() {
        let mut o = observed(100, vec![output("a", 101_000, None)]);
        o.server = Some(canceled_report(server_key(), Some(server_key())));
        let d = decide_in(&record(Side::Source), &o);
        assert!(matches!(d.action, Some(Action::Close(_))));
        assert!(matches!(
            d.status,
            Status::Refund {
                expires_at: 5_100,
                refundable: 101_000,
                matures_at: None,
                ..
            }
        ));
    }

    #[test]
    fn swap_in_closes_to_refund_once_the_csv_matures() {
        let o = observed(243, vec![output("a", 101_000, Some(100))]);
        let d = decide_in(&record(Side::Source), &o);
        assert!(matches!(
            d.status,
            Status::Refund {
                refundable: 101_000,
                matures_at: Some(244),
                ..
            }
        ));
    }

    #[test]
    fn refund_tracks_the_chain_and_never_leaves() {
        let r = with_key(SwapRecord {
            status: Status::Refund {
                expires_at: 9_000,
                refundable: 101_000,
                matures_at: None,
                refunded: vec![],
            },
            ..record(Side::Source)
        });
        let mut deposit = output("a", 101_000, Some(100));
        deposit.spend = Some(spender("refund", None));
        let stray = output("b", 5_000, None);
        let d = decide_in(&r, &observed(150, vec![deposit, stray]));
        assert!(d.action.is_none());
        assert_eq!(
            d.status,
            Status::Refund {
                expires_at: 9_000,
                refundable: 5_000,
                matures_at: None,
                refunded: vec![("refund".into(), false)],
            }
        );
        assert!(!d.status.is_quiet(10_000));
    }

    #[test]
    fn spendable_needs_the_csv_without_the_key() {
        let r = SwapRecord {
            status: Status::Refund {
                expires_at: 0,
                refundable: 0,
                matures_at: None,
                refunded: vec![],
            },
            ..record(Side::Source)
        };
        let o = observed(
            242,
            vec![output("a", 101_000, Some(100)), output("b", 5_000, None)],
        );
        let t = terms(&r);
        assert!(spendable(&r, &t, &o, false).is_empty());
        assert_eq!(spendable(&r, &t, &o, true).len(), 2);
        let o = observed(243, o.outputs);
        assert_eq!(spendable(&r, &t, &o, false).len(), 1);
    }

    #[test]
    fn strays_are_ours_while_the_deposit_is_in_play() {
        let r = record(Side::Source);
        let o = observed(
            300,
            vec![
                output("a", 101_000, Some(100)),
                output("b", 5_000, Some(100)),
            ],
        );
        let utxos = spendable(&r, &terms(&r), &o, false);
        assert_eq!(utxos.len(), 1);
        assert_eq!(utxos[0].txid, "b");
    }

    #[test]
    fn swap_out_closes_canceled_on_the_report_and_only_then() {
        let r = record(Side::Dest);
        let mut o = observed(100, vec![]);
        o.now = 5_000;
        let d = decide_in(&r, &o);
        assert!(matches!(d.action, Some(Action::Send(Message::Cancel))));
        assert!(matches!(d.status, Status::Funding { .. }));

        o.server = Some(Server {
            status: Some(ServerStatus::Out(SwapOutStatus::Canceled {
                error_code: ErrorType::UserCanceled,
                cancel_message: String::new(),
            })),
            key: None,
            preimage: None,
        });
        assert!(matches!(
            decide_in(&r, &o).status,
            Status::Canceled { opening: None }
        ));
    }

    #[test]
    fn swap_out_commits_only_through_the_release_gate() {
        let r = record(Side::Dest);
        let shallow = observed(101, vec![output("open", 100_000, Some(100))]);
        let d = decide_in(&r, &shallow);
        assert!(d.action.is_none());
        assert!(matches!(d.status, Status::Confirming { confs: 2, .. }));

        let deep = observed(102, vec![output("open", 100_000, Some(100))]);
        let d = decide_in(&r, &deep);
        assert!(matches!(d.action, Some(Action::Commit)));
        assert!(matches!(
            d.status,
            Status::CoopKey {
                expires_at: 1_800,
                matures_at: 244
            }
        ));

        let late = observed(100, vec![output("open", 100_000, Some(1))]);
        let d = decide_in(&r, &late);
        assert!(d.action.is_none());
        assert!(matches!(d.status, Status::Confirming { .. }));
    }

    #[test]
    fn swap_out_sends_the_preimage_once_committed_then_waits_for_the_key() {
        let r = committed(record(Side::Dest));
        let o = observed(102, vec![output("open", 100_000, Some(100))]);
        let d = decide_in(&r, &o);
        assert!(matches!(d.action, Some(Action::Send(Message::Preimage))));
        assert!(matches!(
            d.status,
            Status::CoopKey {
                expires_at: 1_700,
                ..
            }
        ));

        let sent = SwapRecord {
            sent: Sent {
                preimage: true,
                ..Sent::default()
            },
            ..r.clone()
        };
        assert!(decide_in(&sent, &o).action.is_none());

        let mut o = o;
        o.now = 1_700;
        assert!(matches!(
            decide_in(&sent, &o).action,
            Some(Action::Spend(Spend::Claim {
                path: Path::Preimage,
                ..
            }))
        ));

        let keyed = with_key(sent);
        assert!(matches!(
            decide_in(&keyed, &o).action,
            Some(Action::Spend(Spend::Claim { path: Path::Keypath { .. }, fee_rate, .. })) if fee_rate == 2.0
        ));
    }

    #[test]
    fn swap_out_caps_the_fee_rate() {
        let r = with_key(record(Side::Dest));
        let mut o = observed(102, vec![output("open", 100_000, Some(100))]);
        o.fee_rate = 500.0;
        assert!(matches!(
            decide_in(&r, &o).action,
            Some(Action::Spend(Spend::Claim { fee_rate, .. })) if fee_rate == 100.0
        ));
    }

    #[test]
    fn swap_out_rebroadcasts_its_claim_and_completes_at_depth() {
        let r = committed(record(Side::Dest));
        let mut opening = output("open", 100_000, Some(100));
        opening.spend = Some(Spender {
            pays_address: true,
            ..spender("claim", None)
        });
        let d = decide_in(&r, &observed(110, vec![opening.clone()]));
        assert!(matches!(
            d.action,
            Some(Action::Spend(Spend::Rebroadcast(_)))
        ));
        assert!(matches!(d.status, Status::Spending { confs: 0, .. }));

        opening.spend.as_mut().unwrap().height = Some(111);
        let d = decide_in(&r, &observed(111, vec![opening.clone()]));
        assert!(d.action.is_none());
        assert!(matches!(d.status, Status::Spending { confs: 1, .. }));

        let d = decide_in(&r, &observed(113, vec![opening]));
        assert!(matches!(
            d.status,
            Status::Completed { payout: Some(ref payout), .. } if payout == "claim"
        ));
    }

    #[test]
    fn swap_out_a_foreign_spend_is_lost_after_commit_and_canceled_before() {
        let mut opening = output("open", 100_000, Some(100));
        opening.spend = Some(spender("refund", Some(250)));
        let o = observed(252, vec![opening]);

        let before = record(Side::Dest);
        assert!(matches!(
            decide_in(&before, &o).status,
            Status::Canceled { opening: Some(_) }
        ));

        let after = committed(before);
        assert!(matches!(decide_in(&after, &o).status, Status::Lost { .. }));
    }

    #[test]
    fn quiet_refund_needs_nothing_at_stake_and_the_window_passed() {
        let settled = Status::Refund {
            expires_at: 100,
            refundable: 0,
            matures_at: None,
            refunded: vec![("a".into(), true)],
        };
        assert!(!settled.is_quiet(99));
        assert!(settled.is_quiet(100));

        let pending = Status::Refund {
            expires_at: 100,
            refundable: 0,
            matures_at: None,
            refunded: vec![("a".into(), false)],
        };
        assert!(!pending.is_quiet(100));
    }
}
