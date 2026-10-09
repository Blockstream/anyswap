//! SwapOut: the hold invoice, the opening it buys, and our claim of it.

use super::{Action, Decision, Message, Path, Spend, funding_deadline};
use crate::{
    driver::{
        config::{ChainConfig, Config},
        observe::{Observed, Output, Spender},
        record::{CancelReason, Status, SwapRecord},
    },
    swap_script::{SpendPath, Terms},
};

/// A SwapOut turns on where the opening is and whether our preimage may have
/// left. Before it leaves nothing is at stake. After it leaves, every pass
/// claims the opening until a claim of it is in a block.
pub(super) fn decide(
    record: &SwapRecord,
    terms: &Terms,
    observed: &Observed,
    config: &Config,
    chain_config: &ChainConfig,
) -> Decision {
    let (opening, _strays) = observed.split(record.receive_amount);
    let Some(opening) = opening else {
        return funding(record, observed, config);
    };
    // Left once tried, answered with the server's key, or seen in a claim of
    // ours on chain, which a restored record learns only from the chain.
    let preimage_left =
        record.sent.preimage || record.dest.server_privkey.is_some() || record.preimage.is_some();
    match &opening.spend {
        Some(spend) if observed.confs(spend.height) >= chain_config.min_confs => {
            spent(opening, spend)
        }
        _ if preimage_left => claiming(record, observed, chain_config, opening),
        _ => confirming(terms, observed, chain_config, opening),
    }
}

/// No opening yet. Past the funding deadline the driver asks the server to
/// cancel, and the swap ends once the server reports it canceled.
fn funding(record: &SwapRecord, observed: &Observed, config: &Config) -> Decision {
    if let Some(cancel) = &record.server_cancel {
        return Decision {
            status: Status::Canceled {
                opening: None,
                reason: CancelReason::ServerCanceled(cancel.clone()),
            },
            action: None,
        };
    }
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

/// The preimage has not left. It goes out once the opening has `min_confs`
/// and is unspent, and only while at least `claim_margin` of the CSV remains,
/// so our claim confirms before the server can refund.
fn confirming(
    terms: &Terms,
    observed: &Observed,
    chain_config: &ChainConfig,
    opening: &Output,
) -> Decision {
    let confs = observed.confs(opening.height);
    let csv_left = terms.csv.saturating_sub(confs);
    let send_preimage = opening.spend.is_none()
        && confs >= chain_config.min_confs
        && csv_left >= chain_config.claim_margin;
    Decision {
        status: Status::Confirming {
            txid: opening.utxo.txid.clone(),
            confs,
            needed: chain_config.min_confs,
        },
        action: send_preimage.then_some(Action::Send(Message::Preimage)),
    }
}

/// The preimage may have left, so the opening must become ours.
fn claiming(
    record: &SwapRecord,
    observed: &Observed,
    chain_config: &ChainConfig,
    opening: &Output,
) -> Decision {
    let our_claim = opening
        .spend
        .as_ref()
        .filter(|spend| spend.path != SpendPath::Csv);
    Decision {
        status: Status::Claiming {
            opening: opening.utxo.txid.clone(),
            claim: our_claim.map(|spend| spend.txid.clone()),
            confs: our_claim.map_or(0, |spend| observed.confs(spend.height)),
        },
        action: claim(record, observed, chain_config, opening),
    }
}

/// A claim of the opening at the current fee rate: by keypath with the
/// server's key, by preimage once the opening has `min_confs`. A spend still
/// in the mempool is outbid, the server's refund at the fee ceiling.
fn claim(
    record: &SwapRecord,
    observed: &Observed,
    chain_config: &ChainConfig,
    opening: &Output,
) -> Option<Action> {
    let pending = opening.spend.as_ref();
    if pending.is_some_and(|spend| spend.height.is_some()) {
        // A spend in a block can no longer be replaced.
        return None;
    }
    let path = match record.dest.server_privkey {
        Some(server_privkey) => Path::Keypath { server_privkey },
        None if observed.confs(opening.height) >= chain_config.min_confs => Path::Preimage,
        None => return None,
    };
    let fee_rate = match pending.map(|spend| spend.path) {
        Some(SpendPath::Csv) => chain_config.max_fee_rate,
        Some(SpendPath::Keypath | SpendPath::Preimage(_)) | None => {
            observed.fee_rate.min(chain_config.max_fee_rate)
        }
    };
    Some(Action::Spend(Spend::Claim {
        utxo: opening.utxo.clone(),
        path,
        fee_rate,
        replaces: pending.map(|spend| spend.txid.clone()),
    }))
}

/// A spend of the opening reached `min_confs`. The server's key alone opens
/// only the CSV leaf, so a spend through it is the server's refund and any
/// other is our claim.
fn spent(opening: &Output, spend: &Spender) -> Decision {
    let opening = opening.utxo.txid.clone();
    let status = match spend.path {
        SpendPath::Keypath | SpendPath::Preimage(_) => Status::Completed {
            opening,
            payout: Some(spend.txid.clone()),
        },
        SpendPath::Csv => Status::Canceled {
            opening: Some(opening),
            reason: CancelReason::ServerRefunded,
        },
    };
    Decision {
        status,
        action: None,
    }
}
