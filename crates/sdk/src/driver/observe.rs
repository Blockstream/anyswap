//! One pass's view of the world: what is under the script and one poll of the
//! server, each verified before anything reads it.

use anyswap_core::{
    api::SwapState, secp, transaction::TxError, types::SwapType, utils::sha256_payment_hash,
};
use bitcoin::secp256k1::SecretKey;

use super::{
    Registered,
    error::DriverError,
    record::{FundState, Height, ServerCancel, SwapRecord, Timestamp},
    server::ServerClient,
};
use crate::{
    swap_in::{SwapIn, SwapInStatus},
    swap_out::{SwapOut, SwapOutStatus},
    swap_script::{Side, SpendPath, SwapScript, Terms, Tx},
    utxo::{self, SwapUtxo},
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
    pub height: Option<Height>,
    pub path: SpendPath,
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
        height.map_or(0, |height| self.tip.saturating_sub(height) + 1)
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
    fee_conf_target: u32,
) -> Result<Observed, DriverError> {
    let tip = chain.client.height().await?;
    let fee_rate = chain.client.fee_rate(fee_conf_target).await?;
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
    for (entry, tx) in &txs {
        for (vin, prev) in tx.inputs().into_iter().enumerate() {
            if let Some(output) = outputs
                .iter_mut()
                .find(|output| output.utxo.outpoint() == prev)
            {
                output.spend = Some(spender(script, tx, vin as u32, entry.height)?);
            }
        }
    }
    // An address history holds only so many mempool transactions, so a spend
    // it misses is asked of the output itself. Only outputs a decision reads
    // are asked, and only the opening's answer is needed to go on, so dust
    // paid to the address cannot stall a pass.
    let amount = match terms.side {
        Side::Source => record.send_amount,
        Side::Dest => record.receive_amount,
    };
    let opening = opening(&outputs, amount).map(|output| output.utxo.outpoint());
    let refund_fee = utxo::refund_fee(terms.chain, fee_rate.min(chain.config.max_fee_rate));
    for output in outputs.iter_mut().filter(|output| output.spend.is_none()) {
        let is_opening = opening == Some(output.utxo.outpoint());
        if !is_opening && !stray_read(record, terms, refund_fee, output) {
            continue;
        }
        match spend_of(terms, script, chain, output).await {
            Ok(spend) => output.spend = spend,
            Err(e) if is_opening => return Err(e),
            Err(_) => {}
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

/// SwapIn: a stray worth its refund fee, or one in a refund of ours. A
/// SwapOut reads no stray.
fn stray_read(
    record: &SwapRecord,
    terms: &Terms,
    refund_fee: Option<u64>,
    output: &Output,
) -> bool {
    let refunding = record.funds.iter().any(|fund| {
        fund.outpoint == output.utxo.outpoint() && matches!(fund.state, FundState::Refunding { .. })
    });
    terms.side == Side::Source && (refund_fee.is_some_and(|fee| output.value > fee) || refunding)
}

/// The spend of `output` as the output itself reports it.
async fn spend_of(
    terms: &Terms,
    script: &SwapScript,
    chain: &Registered,
    output: &Output,
) -> Result<Option<Spender>, DriverError> {
    let outpoint = output.utxo.outpoint();
    let Some(entry) = chain.client.outspend(&outpoint.txid, outpoint.vout).await? else {
        return Ok(None);
    };
    let tx = Tx::decode(terms.chain, &entry.hex)?;
    let Some(vin) = tx.inputs().iter().position(|prev| *prev == outpoint) else {
        return Ok(None);
    };
    Ok(Some(spender(script, &tx, vin as u32, entry.height)?))
}

/// The spend of a swap output by input `vin` of `tx`, its path read from the
/// witness.
fn spender(
    script: &SwapScript,
    tx: &Tx,
    vin: u32,
    height: Option<Height>,
) -> Result<Spender, TxError> {
    let path = script.spend_path(tx, vin).ok_or_else(|| {
        TxError::Decode(format!("input {vin} of {} opens no swap path", tx.txid()))
    })?;
    Ok(Spender {
        txid: tx.txid(),
        height,
        path,
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

    let label = state.status.clone();
    let error_type = state.error_type.clone();
    let cancel_message = state.cancel_message.clone();
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
