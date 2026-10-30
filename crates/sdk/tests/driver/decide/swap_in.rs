use anyswap_sdk::{
    driver::{Action, CancelReason, Message, ServerCancel, Status},
    swap_script::SpendPath,
};

use crate::common::{
    DEADLINE, NOW, PREIMAGE, SEND_AMOUNT, STARTED_AT, SWAP_IN_CSV, confs, invoice, observed,
    opening, opening_txid, output, run, server_key, spend_txid, spent, swap_in, txid,
};

fn funding(expires_at: u64, cancel_sent: bool) -> Status {
    Status::Funding {
        expires_at,
        cancel_sent,
    }
}

fn server_canceled(opening: Option<String>) -> Status {
    Status::Canceled {
        opening,
        reason: CancelReason::ServerCanceled(ServerCancel::default()),
    }
}

fn confirming(confs: u32) -> Status {
    Status::Confirming {
        txid: opening_txid(),
        confs,
        needed: 3,
        max_fee_rate: 100.0,
        fee_rate_spike: None,
    }
}

/// When the server's key arrives and nothing is deposited, the swap is canceled by the server with
/// no opening.
#[test]
fn server_key_without_opening_cancels() {
    let mut record = swap_in();
    record.source.server_privkey = Some(server_key());

    let decision = run(&record, &observed(vec![]));

    assert_eq!(decision.status, server_canceled(None));
    assert!(decision.action.is_none());
}

/// When the server's key arrives with a deposit on chain, the swap is canceled and names the
/// deposit as its opening.
#[test]
fn server_key_with_opening_cancels_with_opening() {
    let mut record = swap_in();
    record.source.server_privkey = Some(server_key());
    let outputs = vec![opening(&record, confs(1))];

    let decision = run(&record, &observed(outputs));

    assert_eq!(decision.status, server_canceled(Some(opening_txid())));
    assert!(decision.action.is_none());
}

/// When both the server's key and a preimage are known, the swap is canceled, not completed.
#[test]
fn server_key_wins_over_preimage() {
    let mut record = swap_in();
    record.source.server_privkey = Some(server_key());
    record.preimage = Some(PREIMAGE);
    let outputs = vec![opening(&record, confs(1))];

    let decision = run(&record, &observed(outputs));

    assert_eq!(decision.status, server_canceled(Some(opening_txid())));
    assert!(decision.action.is_none());
}

/// When the server's key arrives after the deposit's CSV matured, the reason is the server's
/// cancel, not the CSV.
#[test]
fn server_key_wins_over_matured_csv() {
    let mut record = swap_in();
    record.source.server_privkey = Some(server_key());
    let outputs = vec![opening(&record, confs(SWAP_IN_CSV))];

    let decision = run(&record, &observed(outputs));

    assert_eq!(decision.status, server_canceled(Some(opening_txid())));
}

/// When the record holds the server's cancel report, the cancel reason carries it.
#[test]
fn server_cancel_carries_the_stored_report() {
    let mut record = swap_in();
    record.source.server_privkey = Some(server_key());
    let cancel = ServerCancel {
        error_type: Some("Timeout".into()),
        cancel_message: Some("no deposit".into()),
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
}

/// When a preimage is known and the deposit is on chain, the swap completes and sends our refund
/// key.
#[test]
fn preimage_completes_and_sends_refund_key() {
    let mut record = swap_in();
    record.preimage = Some(PREIMAGE);
    let outputs = vec![opening(&record, confs(3))];

    let decision = run(&record, &observed(outputs));

    assert_eq!(
        decision.status,
        Status::Completed {
            opening: opening_txid(),
            payout: None,
            fee: None,
        }
    );
    assert_eq!(decision.action, Some(Action::Send(Message::RefundKey)));
}

/// When the refund key was already sent, the completed swap sends nothing.
#[test]
fn refund_key_goes_out_once() {
    let mut record = swap_in();
    record.preimage = Some(PREIMAGE);
    record.sent.refund_key = true;
    let outputs = vec![opening(&record, confs(3))];

    let decision = run(&record, &observed(outputs));

    assert!(matches!(decision.status, Status::Completed { .. }));
    assert!(decision.action.is_none());
}

/// When the server has spent the deposit, the completed status names that spend as the payout.
#[test]
fn completed_payout_is_the_server_claim() {
    let mut record = swap_in();
    record.preimage = Some(PREIMAGE);
    record.sent.refund_key = true;
    let deposit = opening(&record, confs(3));
    let outputs = vec![spent(deposit, SpendPath::Preimage(PREIMAGE), confs(1))];

    let decision = run(&record, &observed(outputs));

    assert_eq!(
        decision.status,
        Status::Completed {
            opening: opening_txid(),
            payout: Some(spend_txid()),
            fee: None,
        }
    );
}

/// When a preimage is known and the deposit's CSV has matured, the swap completes, not cancels.
#[test]
fn preimage_wins_over_matured_csv() {
    let mut record = swap_in();
    record.preimage = Some(PREIMAGE);
    let outputs = vec![opening(&record, confs(SWAP_IN_CSV))];

    let decision = run(&record, &observed(outputs));

    assert!(matches!(decision.status, Status::Completed { .. }));
}

/// When a preimage is known but nothing is deposited, the swap stays funding.
#[test]
fn preimage_without_opening_is_funding() {
    let mut record = swap_in();
    record.preimage = Some(PREIMAGE);

    let decision = run(&record, &observed(vec![]));

    assert_eq!(decision.status, funding(DEADLINE, false));
    assert!(decision.action.is_none());
}

/// When the deposit has exactly `csv` confirmations and neither key nor preimage is known, the swap
/// is canceled as CSV matured.
#[test]
fn matured_csv_cancels() {
    let record = swap_in();
    let outputs = vec![opening(&record, confs(SWAP_IN_CSV))];

    let decision = run(&record, &observed(outputs));

    assert_eq!(
        decision.status,
        Status::Canceled {
            opening: Some(opening_txid()),
            reason: CancelReason::CsvMatured,
        }
    );
    assert!(decision.action.is_none());
}

/// When the deposit is one confirmation short of the CSV, the swap is still confirming.
#[test]
fn csv_one_block_short_is_confirming() {
    let record = swap_in();
    let outputs = vec![opening(&record, confs(SWAP_IN_CSV - 1))];

    let decision = run(&record, &observed(outputs));

    assert_eq!(decision.status, confirming(SWAP_IN_CSV - 1));
}

/// When the deposit is in the mempool, the swap is confirming with no confirmations.
#[test]
fn mempool_opening_is_confirming_with_no_confs() {
    let record = swap_in();
    let outputs = vec![opening(&record, None)];

    let decision = run(&record, &observed(outputs));

    assert_eq!(decision.status, confirming(0));
    assert!(decision.action.is_none());
}

/// When the deposit is confirmed, the swap is confirming with its txid, confirmations, `min_confs`
/// and `max_fee_rate`, and sends nothing.
#[test]
fn confirmed_opening_is_confirming() {
    let record = swap_in();
    let outputs = vec![opening(&record, confs(2))];

    let decision = run(&record, &observed(outputs));

    assert_eq!(decision.status, confirming(2));
    assert!(decision.action.is_none());
}

/// When the deposit is past `min_confs`, the swap still waits, since paying the invoice is the
/// server's move.
#[test]
fn confirming_past_min_confs_waits_for_the_server() {
    let record = swap_in();
    let outputs = vec![opening(&record, confs(10))];

    let decision = run(&record, &observed(outputs));

    assert_eq!(decision.status, confirming(10));
    assert!(decision.action.is_none());
}

/// When the fee estimate is above `max_fee_rate`, a SwapIn reports no spike.
#[test]
fn fee_rate_spike_is_not_reported() {
    let record = swap_in();
    let mut observed = observed(vec![opening(&record, confs(1))]);
    observed.fee_rate = 500.0;

    let decision = run(&record, &observed);

    assert_eq!(decision.status, confirming(1));
}

/// When nothing is on chain before the deadline, the swap is funding and sends nothing.
#[test]
fn funding_before_deadline_waits() {
    let decision = run(&swap_in(), &observed(vec![]));

    assert_eq!(decision.status, funding(DEADLINE, false));
    assert!(decision.action.is_none());
}

/// When the clock reaches the deadline exactly, the cancel is sent.
#[test]
fn funding_at_deadline_sends_cancel() {
    let mut observed = observed(vec![]);
    observed.now = DEADLINE;

    let decision = run(&swap_in(), &observed);

    assert_eq!(decision.status, funding(DEADLINE, true));
    assert_eq!(decision.action, Some(Action::Send(Message::Cancel)));
}

/// When the cancel was already sent and the deadline has passed, nothing is sent again.
#[test]
fn cancel_goes_out_once() {
    let mut record = swap_in();
    record.sent.cancel = true;
    let mut observed = observed(vec![]);
    observed.now = DEADLINE + 1;

    let decision = run(&record, &observed);

    assert_eq!(decision.status, funding(DEADLINE, true));
    assert!(decision.action.is_none());
}

/// When the user sent a cancel before the deadline, the status shows it sent and nothing more goes
/// out.
#[test]
fn user_cancel_before_deadline_shows_cancel_sent() {
    let mut record = swap_in();
    record.sent.cancel = true;

    let decision = run(&record, &observed(vec![]));

    assert_eq!(decision.status, funding(DEADLINE, true));
    assert!(decision.action.is_none());
}

/// When only deposits of another amount are on chain, even past the CSV, the swap stays funding.
#[test]
fn deposits_of_another_amount_are_not_the_opening() {
    let outputs = vec![
        output(txid(3), SEND_AMOUNT - 1, confs(SWAP_IN_CSV)),
        output(txid(4), SEND_AMOUNT + 1, confs(SWAP_IN_CSV)),
    ];

    let decision = run(&swap_in(), &observed(outputs));

    assert_eq!(decision.status, funding(DEADLINE, false));
}

/// When two passes run at different times before the deadline, they give the same status.
#[test]
fn funding_status_does_not_move_with_the_clock() {
    let record = swap_in();
    let mut later = observed(vec![]);
    later.now = NOW + 1_000;

    let first = run(&record, &observed(vec![]));
    let second = run(&record, &later);

    assert_eq!(first.status, second.status);
}

/// The deadline is `min_confs + 1` blocks before the claim invoice expires.
#[test]
fn deadline_leaves_confirm_room_before_invoice_expiry() {
    let mut record = swap_in();
    record.dest.invoice = Some(invoice(STARTED_AT, 3_000));

    let decision = run(&record, &observed(vec![]));

    assert_eq!(decision.status, funding(STARTED_AT + 3_000 - 2_400, false));
}

/// When the invoice outlives `funding_timeout`, the deadline is `funding_timeout`.
#[test]
fn deadline_is_capped_at_funding_timeout() {
    let mut record = swap_in();
    record.dest.invoice = Some(invoice(STARTED_AT, 200_000));

    let decision = run(&record, &observed(vec![]));

    assert_eq!(decision.status, funding(DEADLINE, false));
}

/// When the invoice expires sooner than the confirm room, the deadline falls before the start and
/// the first pass sends the cancel.
#[test]
fn invoice_shorter_than_confirm_room_cancels_at_once() {
    let mut record = swap_in();
    record.dest.invoice = Some(invoice(STARTED_AT, 1_000));
    let mut observed = observed(vec![]);
    observed.now = STARTED_AT;

    let decision = run(&record, &observed);

    assert_eq!(decision.status, funding(STARTED_AT + 1_000 - 2_400, true));
    assert_eq!(decision.action, Some(Action::Send(Message::Cancel)));
}

/// When the confirm room reaches back past the epoch, the deadline is zero instead of underflowing.
#[test]
fn deadline_saturates_at_zero() {
    let mut record = swap_in();
    record.started_at = 100;
    record.dest.invoice = Some(invoice(100, 1_000));

    let decision = run(&record, &observed(vec![]));

    assert_eq!(decision.status, funding(0, true));
}
