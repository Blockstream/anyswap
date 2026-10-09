//! SwapIn: the deposit, the invoice it pays for, and the refund of what is ours.

use super::{Action, Decision, Message, funding_deadline};
use crate::{
    driver::{
        config::{ChainConfig, Config},
        observe::{Observed, Output, Spender},
        record::{CancelReason, Fund, FundState, Status, SwapRecord},
    },
    swap_script::{SpendPath, Terms},
    utxo,
};

/// A SwapIn turns on whose the opening is: the server's once our preimage
/// verified, ours once its key verified or the CSV matured. The key goes
/// first: once the server gave the deposit up, a preimage arriving after
/// cannot take it back.
pub(super) fn decide(
    record: &SwapRecord,
    terms: &Terms,
    observed: &Observed,
    config: &Config,
    chain_config: &ChainConfig,
) -> Decision {
    let (opening, _strays) = observed.split(record.send_amount);
    match (record.source.server_privkey, record.preimage, opening) {
        (Some(_), _, opening) => {
            let cancel = record.server_cancel.clone().unwrap_or_default();
            canceled(opening, CancelReason::ServerCanceled(cancel))
        }
        (None, Some(_), Some(opening)) => completed(record, opening),
        (None, _, Some(opening)) if observed.confs(opening.height) >= terms.csv => {
            canceled(Some(opening), CancelReason::CsvMatured)
        }
        (None, _, Some(opening)) => confirming(observed, chain_config, opening),
        (None, _, None) => funding(record, observed, config),
    }
}

/// No deposit yet. Past the funding deadline the driver asks the server to
/// cancel, and the swap ends once the server's key arrives with its answer.
fn funding(record: &SwapRecord, observed: &Observed, config: &Config) -> Decision {
    let expires_at = funding_deadline(record, config);
    let send_cancel = observed.now >= expires_at && !record.sent.cancel;
    Decision {
        status: Status::Funding {
            expires_at,
            cancel_sent: record.sent.cancel || send_cancel,
        },
        action: send_cancel.then_some(Action::Send(Message::Cancel)),
    }
}

/// A deposit is under the script, and paying the invoice is the server's move.
fn confirming(observed: &Observed, chain_config: &ChainConfig, opening: &Output) -> Decision {
    Decision {
        status: Status::Confirming {
            txid: opening.utxo.txid.clone(),
            confs: observed.confs(opening.height),
            needed: chain_config.min_confs,
        },
        action: None,
    }
}

/// The preimage verified, so the invoice is paid and the deposit is the
/// server's. Our refund key goes out once, so it can claim by the keypath.
fn completed(record: &SwapRecord, opening: &Output) -> Decision {
    Decision {
        status: Status::Completed {
            opening: opening.utxo.txid.clone(),
            payout: opening.spend.as_ref().map(|spend| spend.txid.clone()),
        },
        action: (!record.sent.refund_key).then_some(Action::Send(Message::RefundKey)),
    }
}

/// The swap did not happen, and the deposit is ours with the rest of `funds`.
fn canceled(opening: Option<&Output>, reason: CancelReason) -> Decision {
    Decision {
        status: Status::Canceled {
            opening: opening.map(|opening| opening.utxo.txid.clone()),
            reason,
        },
        action: None,
    }
}

/// What under the script is ours: every stray, and the opening too once the
/// swap is canceled. Before that the opening is in play, and once completed
/// it is the server's.
pub(super) fn funds(
    record: &SwapRecord,
    terms: &Terms,
    observed: &Observed,
    chain_config: &ChainConfig,
    status: &Status,
) -> Vec<Fund> {
    let (opening, strays) = observed.split(record.send_amount);
    let ours = match status {
        Status::Canceled { .. } => strays.into_iter().chain(opening).collect(),
        _ => strays,
    };
    let fee_rate = observed.fee_rate.min(chain_config.max_fee_rate);
    let refund_fee = utxo::refund_fee(terms.chain, fee_rate);
    ours.into_iter()
        .filter_map(|output| {
            let state = state(record, terms, observed, refund_fee, output)?;
            Some(Fund {
                outpoint: output.utxo.outpoint(),
                amount: output.value,
                state,
            })
        })
        .collect()
}

/// Where the refund of one output of ours stands, `None` once the server's
/// own path took it.
fn state(
    record: &SwapRecord,
    terms: &Terms,
    observed: &Observed,
    refund_fee: Option<u64>,
    output: &Output,
) -> Option<FundState> {
    Some(match &output.spend {
        Some(spend) if !our_refund(record, spend) => return None,
        Some(spend) if spend.height.is_none() => FundState::Refunding {
            txid: spend.txid.clone(),
        },
        Some(spend) => FundState::Refunded {
            txid: spend.txid.clone(),
        },
        None if record.refund_keypath().is_none() && observed.confs(output.height) < terms.csv => {
            FundState::Locked {
                at: output.height.map(|height| height + terms.csv),
            }
        }
        None if refund_fee.is_none_or(|fee| output.value <= fee) => FundState::Dust,
        None => FundState::Refundable,
    })
}

/// The CSV leaf is ours and the preimage leaf the server's. The keypath is
/// ours only while our refund key is home, since we never take it after.
fn our_refund(record: &SwapRecord, spend: &Spender) -> bool {
    match spend.path {
        SpendPath::Csv => true,
        SpendPath::Keypath => !record.sent.refund_key,
        SpendPath::Preimage(_) => false,
    }
}
