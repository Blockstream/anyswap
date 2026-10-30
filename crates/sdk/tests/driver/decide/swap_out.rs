use anyswap_sdk::{
    driver::{Action, CancelReason, ChainConfig, Message, Path, ServerCancel, Spend, Status},
    swap_script::SpendPath,
    utxo::SwapUtxo,
};

use crate::common::{
    DEADLINE, FEE_RATE, PREIMAGE, RECEIVE_AMOUNT, SEND_AMOUNT, SPEND_FEE, STARTED_AT, SWAP_OUT_CSV,
    confs, invoice, observed, opening, opening_txid, output, run, run_with, server_key, spend_txid,
    spent, swap_out, txid,
};

fn funding(expires_at: u64, cancel_sent: bool) -> Status {
    Status::Funding {
        expires_at,
        cancel_sent,
    }
}

fn confirming(confs: u32, fee_rate_spike: Option<f64>) -> Status {
    Status::Confirming {
        txid: opening_txid(),
        confs,
        needed: 3,
        max_fee_rate: 100.0,
        fee_rate_spike,
    }
}

fn claiming(claim: Option<String>, confs: u32, fee: Option<u64>) -> Status {
    Status::Claiming {
        opening: opening_txid(),
        claim,
        confs,
        fee,
    }
}

fn keypath() -> Path {
    Path::Keypath {
        server_privkey: server_key(),
    }
}

fn claim_of(path: Path, fee_rate: f64, replaces: Option<String>) -> Option<Action> {
    Some(Action::Spend(Spend::Claim {
        utxo: SwapUtxo::bitcoin(opening_txid(), 0, RECEIVE_AMOUNT),
        path,
        fee_rate,
        replaces,
    }))
}

/// When the server reports a cancel and there is no opening, the swap is canceled with its report,
/// before any deadline.
#[test]
fn server_cancel_before_opening_cancels() {
    let mut record = swap_out();
    let cancel = ServerCancel {
        error_type: Some("Timeout".into()),
        cancel_message: Some("not paid".into()),
    };
    record.server_cancel = Some(cancel.clone());

    let decision = run(&record, &observed(vec![]));

    assert_eq!(
        decision.status,
        Status::Canceled {
            opening: None,
            reason: CancelReason::ServerCanceled(cancel),
        }
    );
    assert!(decision.action.is_none());
}

/// When nothing is on chain before the deadline, the swap is funding and sends nothing.
#[test]
fn funding_before_deadline_waits() {
    let decision = run(&swap_out(), &observed(vec![]));

    assert_eq!(decision.status, funding(DEADLINE, false));
    assert!(decision.action.is_none());
}

/// When the clock reaches the deadline exactly, the cancel is sent.
#[test]
fn funding_at_deadline_sends_cancel() {
    let mut observed = observed(vec![]);
    observed.now = DEADLINE;

    let decision = run(&swap_out(), &observed);

    assert_eq!(decision.status, funding(DEADLINE, true));
    assert_eq!(decision.action, Some(Action::Send(Message::Cancel)));
}

/// When the cancel was already sent and the deadline has passed, nothing is sent again.
#[test]
fn cancel_goes_out_once() {
    let mut record = swap_out();
    record.sent.cancel = true;
    let mut observed = observed(vec![]);
    observed.now = DEADLINE + 1;

    let decision = run(&record, &observed);

    assert_eq!(decision.status, funding(DEADLINE, true));
    assert!(decision.action.is_none());
}

/// When only an output of `send_amount` is on chain, the swap stays funding, since the opening is
/// matched on `receive_amount`.
#[test]
fn send_amount_output_is_not_the_opening() {
    let outputs = vec![output(txid(3), SEND_AMOUNT, confs(3))];

    let decision = run(&swap_out(), &observed(outputs));

    assert_eq!(decision.status, funding(DEADLINE, false));
}

/// The deadline is the hold invoice's expiry, however far past `funding_timeout`.
#[test]
fn deadline_is_the_hold_invoice_expiry() {
    let mut record = swap_out();
    record.source.invoice = Some(invoice(STARTED_AT, 200_000));

    let decision = run(&record, &observed(vec![]));

    assert_eq!(decision.status, funding(STARTED_AT + 200_000, false));
}

/// When a keypath spend of the opening reaches `min_confs`, the swap completes with its txid and
/// fee.
#[test]
fn confirmed_keypath_spend_completes() {
    let record = swap_out();
    let outputs = vec![spent(
        opening(&record, confs(10)),
        SpendPath::Keypath,
        confs(3),
    )];

    let decision = run(&record, &observed(outputs));

    assert_eq!(
        decision.status,
        Status::Completed {
            opening: opening_txid(),
            payout: Some(spend_txid()),
            fee: Some(SPEND_FEE),
        }
    );
    assert!(decision.action.is_none());
}

/// When a preimage spend of the opening reaches `min_confs`, the swap completes.
#[test]
fn confirmed_preimage_spend_completes() {
    let record = swap_out();
    let outputs = vec![spent(
        opening(&record, confs(10)),
        SpendPath::Preimage(PREIMAGE),
        confs(3),
    )];

    let decision = run(&record, &observed(outputs));

    assert!(matches!(decision.status, Status::Completed { .. }));
}

/// When a CSV spend of the opening reaches `min_confs`, the swap is canceled as refunded by the
/// server.
#[test]
fn confirmed_csv_spend_is_server_refund() {
    let record = swap_out();
    let outputs = vec![spent(
        opening(&record, confs(SWAP_OUT_CSV + 3)),
        SpendPath::Csv,
        confs(3),
    )];

    let decision = run(&record, &observed(outputs));

    assert_eq!(
        decision.status,
        Status::Canceled {
            opening: Some(opening_txid()),
            reason: CancelReason::ServerRefunded,
        }
    );
    assert!(decision.action.is_none());
}

/// When the server's refund is short of `min_confs` and the preimage was never sent, the swap is
/// canceled as claim window closed.
#[test]
fn server_refund_below_min_confs_closes_claim_window() {
    let record = swap_out();
    let height = confs(SWAP_OUT_CSV);
    let outputs = vec![spent(opening(&record, height), SpendPath::Csv, confs(2))];

    let decision = run(&record, &observed(outputs));

    assert_eq!(
        decision.status,
        Status::Canceled {
            opening: Some(opening_txid()),
            reason: CancelReason::ClaimWindowClosed {
                at: height.unwrap() + SWAP_OUT_CSV,
            },
        }
    );
}

/// When the preimage was sent and a spend of the opening reaches `min_confs`, the swap completes
/// and claims nothing.
#[test]
fn final_spend_wins_over_claiming() {
    let mut record = swap_out();
    record.sent.preimage = true;
    let outputs = vec![spent(
        opening(&record, confs(10)),
        SpendPath::Keypath,
        confs(3),
    )];

    let decision = run(&record, &observed(outputs));

    assert!(matches!(decision.status, Status::Completed { .. }));
    assert!(decision.action.is_none());
}

/// When the preimage was sent and the opening is unspent, the swap claims it by preimage at the fee
/// estimate.
#[test]
fn sent_preimage_claims_by_preimage() {
    let mut record = swap_out();
    record.sent.preimage = true;
    let outputs = vec![opening(&record, confs(3))];

    let decision = run(&record, &observed(outputs));

    assert_eq!(decision.status, claiming(None, 0, None));
    assert_eq!(decision.action, claim_of(Path::Preimage, FEE_RATE, None));
}

/// When the server's key arrives and the preimage was never sent, the swap claims by keypath.
#[test]
fn server_key_alone_claims_by_keypath() {
    let mut record = swap_out();
    record.dest.server_privkey = Some(server_key());
    let outputs = vec![opening(&record, confs(3))];

    let decision = run(&record, &observed(outputs));

    assert_eq!(decision.status, claiming(None, 0, None));
    assert_eq!(decision.action, claim_of(keypath(), FEE_RATE, None));
}

/// When the preimage is known from the chain, as on a restored record, the swap claims by preimage.
#[test]
fn preimage_seen_on_chain_claims_by_preimage() {
    let mut record = swap_out();
    record.preimage = Some(PREIMAGE);
    let outputs = vec![opening(&record, confs(3))];

    let decision = run(&record, &observed(outputs));

    assert_eq!(decision.status, claiming(None, 0, None));
    assert_eq!(decision.action, claim_of(Path::Preimage, FEE_RATE, None));
}

/// When our claim is in the mempool, the status shows it and a new claim replaces it at the current
/// rate.
#[test]
fn pending_claim_is_reported_and_replaced() {
    let mut record = swap_out();
    record.sent.preimage = true;
    let outputs = vec![spent(
        opening(&record, confs(3)),
        SpendPath::Preimage(PREIMAGE),
        None,
    )];

    let decision = run(&record, &observed(outputs));

    assert_eq!(
        decision.status,
        claiming(Some(spend_txid()), 0, Some(SPEND_FEE))
    );
    assert_eq!(
        decision.action,
        claim_of(Path::Preimage, FEE_RATE, Some(spend_txid()))
    );
}

/// When our claim is in a block short of `min_confs`, the status shows its confirmations and
/// nothing is built.
#[test]
fn claim_in_block_is_not_replaced() {
    let mut record = swap_out();
    record.sent.preimage = true;
    let outputs = vec![spent(
        opening(&record, confs(3)),
        SpendPath::Preimage(PREIMAGE),
        confs(1),
    )];

    let decision = run(&record, &observed(outputs));

    assert_eq!(
        decision.status,
        claiming(Some(spend_txid()), 1, Some(SPEND_FEE))
    );
    assert!(decision.action.is_none());
}

/// When the server's refund is in the mempool, the status shows no claim of ours.
#[test]
fn pending_server_refund_is_not_our_claim() {
    let mut record = swap_out();
    record.sent.preimage = true;
    let outputs = vec![spent(opening(&record, confs(3)), SpendPath::Csv, None)];

    let decision = run(&record, &observed(outputs));

    assert_eq!(decision.status, claiming(None, 0, None));
}

/// When the server's refund is in a block, nothing is built, since it can no longer be replaced.
#[test]
fn server_refund_in_block_is_not_outbid() {
    let mut record = swap_out();
    record.sent.preimage = true;
    let outputs = vec![spent(opening(&record, confs(3)), SpendPath::Csv, confs(1))];

    let decision = run(&record, &observed(outputs));

    assert_eq!(decision.status, claiming(None, 0, None));
    assert!(decision.action.is_none());
}

/// When we hold the server's key and our keypath claim is in the mempool, it is replaced by
/// keypath.
#[test]
fn pending_keypath_claim_is_replaced_by_keypath() {
    let mut record = swap_out();
    record.dest.server_privkey = Some(server_key());
    let outputs = vec![spent(opening(&record, confs(3)), SpendPath::Keypath, None)];

    let decision = run(&record, &observed(outputs));

    assert_eq!(
        decision.action,
        claim_of(keypath(), FEE_RATE, Some(spend_txid()))
    );
}

/// When we hold the server's key and our preimage claim is in the mempool, it is replaced by
/// preimage, since a smaller keypath claim pays less in all and can be refused.
#[test]
fn pending_preimage_claim_is_replaced_by_preimage_despite_key() {
    let mut record = swap_out();
    record.dest.server_privkey = Some(server_key());
    record.sent.preimage = true;
    let outputs = vec![spent(
        opening(&record, confs(3)),
        SpendPath::Preimage(PREIMAGE),
        None,
    )];

    let decision = run(&record, &observed(outputs));

    assert_eq!(
        decision.action,
        claim_of(Path::Preimage, FEE_RATE, Some(spend_txid()))
    );
}

/// When the preimage was sent and the server's refund is in the mempool, it is outbid by preimage
/// at `max_fee_rate`.
#[test]
fn pending_server_refund_is_outbid_by_preimage_at_max_fee_rate() {
    let mut record = swap_out();
    record.dest.server_privkey = Some(server_key());
    record.sent.preimage = true;
    let outputs = vec![spent(opening(&record, confs(3)), SpendPath::Csv, None)];

    let decision = run(&record, &observed(outputs));

    assert_eq!(
        decision.action,
        claim_of(Path::Preimage, 100.0, Some(spend_txid()))
    );
}

/// When the opening is short of `min_confs` and we hold the server's key, a pending preimage claim
/// is replaced by keypath.
#[test]
fn key_replaces_pending_preimage_claim_on_shallow_opening() {
    let mut record = swap_out();
    record.dest.server_privkey = Some(server_key());
    record.sent.preimage = true;
    let outputs = vec![spent(
        opening(&record, confs(2)),
        SpendPath::Preimage(PREIMAGE),
        None,
    )];

    let decision = run(&record, &observed(outputs));

    assert_eq!(
        decision.action,
        claim_of(keypath(), FEE_RATE, Some(spend_txid()))
    );
}

/// When we hold the server's key, the preimage was never sent and the server's refund is in the
/// mempool, it is outbid by keypath at `max_fee_rate`.
#[test]
fn key_outbids_server_refund_by_keypath_when_preimage_never_left() {
    let mut record = swap_out();
    record.dest.server_privkey = Some(server_key());
    let outputs = vec![spent(opening(&record, confs(3)), SpendPath::Csv, None)];

    let decision = run(&record, &observed(outputs));

    assert_eq!(
        decision.action,
        claim_of(keypath(), 100.0, Some(spend_txid()))
    );
}

/// When the preimage was sent but the opening is back in the mempool and we hold no key, the swap
/// stays claiming and builds nothing.
#[test]
fn shallow_opening_without_key_waits_to_claim() {
    let mut record = swap_out();
    record.sent.preimage = true;
    let outputs = vec![opening(&record, None)];

    let decision = run(&record, &observed(outputs));

    assert_eq!(decision.status, claiming(None, 0, None));
    assert!(decision.action.is_none());
}

/// When the fee estimate is above `max_fee_rate`, the claim pays `max_fee_rate`.
#[test]
fn claim_fee_rate_is_capped_at_max() {
    let mut record = swap_out();
    record.sent.preimage = true;
    let mut observed = observed(vec![opening(&record, confs(3))]);
    observed.fee_rate = 150.0;

    let decision = run(&record, &observed);

    assert_eq!(decision.action, claim_of(Path::Preimage, 100.0, None));
}

/// When the opening has exactly `min_confs`, exactly `claim_margin` of the CSV is left and the
/// estimate equals `max_fee_rate`, the preimage is sent.
#[test]
fn preimage_goes_out_at_every_inclusive_bound() {
    let mut record = swap_out();
    record.dest.csv = Some(3 + 252);
    let mut observed = observed(vec![opening(&record, confs(3))]);
    observed.fee_rate = 100.0;

    let decision = run(&record, &observed);

    assert_eq!(decision.status, confirming(3, None));
    assert_eq!(decision.action, Some(Action::Send(Message::Preimage)));
}

/// When the opening is one confirmation short of `min_confs`, the preimage is held.
#[test]
fn opening_below_min_confs_holds_preimage() {
    let record = swap_out();
    let outputs = vec![opening(&record, confs(2))];

    let decision = run(&record, &observed(outputs));

    assert_eq!(decision.status, confirming(2, None));
    assert!(decision.action.is_none());
}

/// When the fee estimate is above `max_fee_rate`, the preimage is held and the spike reported.
#[test]
fn fee_rate_spike_holds_preimage() {
    let record = swap_out();
    let mut observed = observed(vec![opening(&record, confs(3))]);
    observed.fee_rate = 150.0;

    let decision = run(&record, &observed);

    assert_eq!(decision.status, confirming(3, Some(150.0)));
    assert!(decision.action.is_none());
}

/// When the estimate spikes before the opening has `min_confs`, the spike is already reported.
#[test]
fn fee_rate_spike_is_reported_below_min_confs() {
    let record = swap_out();
    let mut observed = observed(vec![opening(&record, confs(1))]);
    observed.fee_rate = 150.0;

    let decision = run(&record, &observed);

    assert_eq!(decision.status, confirming(1, Some(150.0)));
}

/// When less than `claim_margin` of the CSV is left, the swap is canceled as claim window closed at
/// the opening's height plus `csv`.
#[test]
fn claim_window_closed_cancels() {
    let mut record = swap_out();
    record.dest.csv = Some(3 + 252);
    let height = confs(4);
    let outputs = vec![opening(&record, height)];

    let decision = run(&record, &observed(outputs));

    assert_eq!(
        decision.status,
        Status::Canceled {
            opening: Some(opening_txid()),
            reason: CancelReason::ClaimWindowClosed {
                at: height.unwrap() + 255,
            },
        }
    );
    assert!(decision.action.is_none());
}

/// When the claim window has closed and the fee rate spikes, the swap is canceled as claim window
/// closed.
#[test]
fn closed_claim_window_wins_over_fee_rate_spike() {
    let mut record = swap_out();
    record.dest.csv = Some(3 + 252);
    let mut observed = observed(vec![opening(&record, confs(4))]);
    observed.fee_rate = 150.0;

    let decision = run(&record, &observed);

    assert!(matches!(
        decision.status,
        Status::Canceled {
            reason: CancelReason::ClaimWindowClosed { .. },
            ..
        }
    ));
}

/// When the opening is in the mempool, the claim window stays open however short the CSV, since it
/// has not started.
#[test]
fn mempool_opening_never_closes_claim_window() {
    let record = swap_out();
    let chain_config = ChainConfig {
        claim_margin: SWAP_OUT_CSV + 1,
        ..ChainConfig::bitcoin()
    };
    let outputs = vec![opening(&record, None)];

    let decision = run_with(&record, &observed(outputs), &chain_config);

    assert_eq!(decision.status, confirming(0, None));
}

/// When the opening is already spent in the mempool and the preimage was never sent, the preimage
/// is held.
#[test]
fn spent_opening_holds_preimage() {
    let record = swap_out();
    let outputs = vec![spent(opening(&record, confs(3)), SpendPath::Csv, None)];

    let decision = run(&record, &observed(outputs));

    assert_eq!(decision.status, confirming(3, None));
    assert!(decision.action.is_none());
}
