//! One pass's view of the world: what is under the script and one poll of the
//! server, each verified before anything reads it.

use std::{cmp::Reverse, collections::HashMap};

use anyswap_core::{
    api::SwapState,
    secp,
    transaction::TxError,
    types::{Outpoint, SwapType},
    utils::sha256_payment_hash,
};
use bitcoin::secp256k1::SecretKey;

use super::{
    Registered,
    config::Config,
    error::DriverError,
    record::{FundState, Height, ServerCancel, SwapRecord, Timestamp},
    rt::bounded,
    server::ServerClient,
};
use crate::{
    swap_in::{SwapIn, SwapInStatus},
    swap_out::{SwapOut, SwapOutStatus},
    swap_script::{Side, SpendPath, SwapScript, Terms, Tx},
    utxo::{self, SwapUtxo},
};

/// The most SwapIn funds a pass reads the spends of and a refund takes, the
/// most valuable first. One refund is in the mempool at a time, so a pass can
/// read every fund in it. Honest use leaves far fewer under one script, so a
/// pile of strays costs a pass no more than this and comes back a block apart.
pub(crate) const FUNDS_PER_PASS: usize = 8;

/// One pass's view of the world, verified; nothing in it is left to believe.
#[derive(Clone, Debug)]
pub struct Observed {
    pub now: Timestamp,
    pub tip: Height,
    /// The chain's estimate at `fee_conf_target`, in sat/vB, at most
    /// `fee_rate_cache_ttl` old.
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
    pub height: Option<Height>,
    pub path: SpendPath,
    /// Its miner fee, `None` on Bitcoin when it spends an output the pass
    /// did not see under the script.
    pub fee: Option<u64>,
}

/// One successful poll of the server, each part taken on its own terms, so a
/// status we cannot parse costs the parse and nothing else.
#[derive(Clone, Debug)]
pub struct Server {
    /// The status as the server names it.
    pub label: String,
    /// Its cancel, once the report parsed as `Canceled`.
    pub cancel: Option<ServerCancel>,
    /// The report as parsed; `None` when it spoke a status we do not know.
    pub status: Option<Report>,
    /// Its key, verified against its pubkey in the terms.
    pub key: Option<SecretKey>,
    /// Its preimage, verified against `payment_hash`.
    pub preimage: Option<[u8; 32]>,
}

/// The server's report of a swap, as the SDK's view of its state parses it.
#[derive(Clone, Debug)]
pub enum Report {
    In(SwapInStatus),
    Out(SwapOutStatus),
}

impl Observed {
    pub fn confs(&self, height: Option<Height>) -> u32 {
        confs(self.tip, height)
    }

    /// The opening and the strays. The opening is the oldest output paying
    /// exactly `amount`, and everything else is a stray, a later payment of
    /// the same amount included, so it cannot hide the opening.
    pub fn split(&self, amount: u64) -> (Option<&Output>, Vec<&Output>) {
        let opening = opening(&self.outputs, amount);
        let opening_outpoint = opening.map(|opening| opening.utxo.outpoint());
        let strays = self
            .outputs
            .iter()
            .filter(|output| Some(output.utxo.outpoint()) != opening_outpoint)
            .collect();
        (opening, strays)
    }

    pub fn server_privkey(&self) -> Option<SecretKey> {
        self.server.as_ref()?.key
    }

    /// A verified preimage from either source.
    pub fn preimage(&self) -> Option<[u8; 32]> {
        self.server
            .as_ref()
            .and_then(|server| server.preimage)
            .or_else(|| {
                self.outputs
                    .iter()
                    .find_map(|output| match output.spend.as_ref()?.path {
                        SpendPath::Preimage(preimage) => Some(preimage),
                        SpendPath::Keypath | SpendPath::Csv => None,
                    })
            })
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
    config: &Config,
) -> Result<Observed, DriverError> {
    let tip = chain.client.height().await?;
    let fee_rate = chain
        .fee_rate(config.fee_conf_target, config.fee_rate_cache_ttl)
        .await?;
    let history = chain.client.address_history(&script.address()).await?;
    let txs = history
        .into_iter()
        .map(|entry| Tx::decode(terms.chain, &entry.hex).map(|tx| (entry, tx)))
        .collect::<Result<Vec<_>, _>>()?;

    let mut outputs: Vec<Output> = txs
        .iter()
        .flat_map(|(entry, tx)| {
            script.outputs(tx).into_iter().map(|utxo| Output {
                value: script.value(&utxo).unwrap_or_default(),
                utxo,
                height: entry.height,
                spend: None,
            })
        })
        .collect();
    let values: HashMap<Outpoint, u64> = outputs
        .iter()
        .map(|output| (output.utxo.outpoint(), output.value))
        .collect();
    for (entry, tx) in &txs {
        for (vin, prev) in tx.inputs().into_iter().enumerate() {
            if let Some(output) = outputs
                .iter_mut()
                .find(|output| output.utxo.outpoint() == prev)
            {
                output.spend = Some(spender(script, tx, vin as u32, entry.height, &values)?);
            }
        }
    }
    // An address history holds only so many mempool transactions, so a spend
    // it misses is asked of the output itself. Only outputs a decision reads
    // are asked, at most `FUNDS_PER_PASS` strays with those in our refund
    // first, and only the opening's answer is needed to go on, so dust paid
    // to the address cannot stall a pass.
    let amount = match terms.side {
        Side::Source => record.send_amount,
        Side::Dest => record.receive_amount,
    };
    let opening = opening(&outputs, amount).map(|output| output.utxo.outpoint());
    let refund_fee = utxo::refund_fee(terms.chain, fee_rate.min(chain.config.max_fee_rate));
    let (openings, mut strays): (Vec<_>, Vec<_>) = outputs
        .iter_mut()
        .filter(|output| output.spend.is_none())
        .partition(|output| opening == Some(output.utxo.outpoint()));
    strays.retain(|output| stray_read(record, terms, tip, refund_fee, output));
    strays.sort_by_key(|output| (!refunding(record, output), Reverse(output.value)));
    strays.truncate(FUNDS_PER_PASS);
    for output in openings {
        output.spend = spend_of(terms, script, chain, output, &values).await?;
    }
    for output in strays {
        if let Ok(spend) = spend_of(terms, script, chain, output, &values).await {
            output.spend = spend;
        }
    }
    outputs.sort_by(|a, b| {
        let key = |output: &Output| {
            (
                output.height.unwrap_or(Height::MAX),
                output.utxo.txid.clone(),
                output.utxo.vout,
            )
        };
        key(a).cmp(&key(b))
    });

    let server = bounded(config.server_timeout, client.get_swap(record.swap_id))
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

/// The oldest output paying exactly `amount`: earliest block first, the
/// mempool last, ties by txid and vout.
fn opening(outputs: &[Output], amount: u64) -> Option<&Output> {
    outputs
        .iter()
        .filter(|output| output.value == amount)
        .min_by_key(|output| {
            let height = output.height.unwrap_or(Height::MAX);
            (height, output.utxo.txid.clone(), output.utxo.vout)
        })
}

/// Confirmations at `tip` of a transaction at `height`, none in the mempool.
fn confs(tip: Height, height: Option<Height>) -> u32 {
    height.map_or(0, |height| tip.saturating_sub(height) + 1)
}

/// SwapIn: a stray in a refund of ours, or one worth its refund fee and not
/// locked, since nothing acts on a locked one. A SwapOut reads no stray.
fn stray_read(
    record: &SwapRecord,
    terms: &Terms,
    tip: Height,
    refund_fee: Option<u64>,
    output: &Output,
) -> bool {
    let refundable = refund_fee.is_some_and(|fee| output.value > fee)
        && !record.locked(terms.csv, confs(tip, output.height));
    terms.side == Side::Source && (refundable || refunding(record, output))
}

/// Whether the last pass saw `output` in a refund of ours in the mempool.
fn refunding(record: &SwapRecord, output: &Output) -> bool {
    record.funds.iter().any(|fund| {
        fund.outpoint == output.utxo.outpoint() && matches!(fund.state, FundState::Refunding { .. })
    })
}

/// The spend of `output` as the output itself reports it.
async fn spend_of(
    terms: &Terms,
    script: &SwapScript,
    chain: &Registered,
    output: &Output,
    values: &HashMap<Outpoint, u64>,
) -> Result<Option<Spender>, DriverError> {
    let outpoint = output.utxo.outpoint();
    let Some(entry) = chain.client.outspend(&outpoint.txid, outpoint.vout).await? else {
        return Ok(None);
    };
    let tx = Tx::decode(terms.chain, &entry.hex)?;
    let Some(vin) = tx.inputs().iter().position(|prev| *prev == outpoint) else {
        return Ok(None);
    };
    Ok(Some(spender(
        script,
        &tx,
        vin as u32,
        entry.height,
        values,
    )?))
}

/// The spend of a swap output by input `vin` of `tx`, its path read from the
/// witness and its fee from `values`, the swap outputs' own.
fn spender(
    script: &SwapScript,
    tx: &Tx,
    vin: u32,
    height: Option<Height>,
    values: &HashMap<Outpoint, u64>,
) -> Result<Spender, TxError> {
    let path = script.spend_path(tx, vin).ok_or_else(|| {
        TxError::Decode(format!("input {vin} of {} opens no swap path", tx.txid()))
    })?;
    Ok(Spender {
        txid: tx.txid(),
        height,
        path,
        fee: tx.fee(|prev| values.get(prev).copied()),
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

    let label = clip(state.status.clone(), 64);
    let error_type = state.error_type.clone();
    let cancel_message = state.cancel_message.clone().map(|text| clip(text, 256));
    let status = match terms.side {
        Side::Source => SwapIn::try_from(state)
            .ok()
            .map(|swap| Report::In(swap.status)),
        Side::Dest => SwapOut::try_from(state)
            .ok()
            .map(|swap| Report::Out(swap.status)),
    };
    let canceled = matches!(
        status,
        Some(
            Report::In(SwapInStatus::Canceled { .. }) | Report::Out(SwapOutStatus::Canceled { .. })
        )
    );
    let cancel = canceled.then_some(ServerCancel {
        error_type,
        cancel_message,
    });

    Server {
        label,
        cancel,
        status,
        key,
        preimage,
    }
}

/// Server text cut to `max` bytes, so a record stays small whatever the
/// server sends.
fn clip(mut text: String, max: usize) -> String {
    text.truncate(text.floor_char_boundary(max));
    text
}
