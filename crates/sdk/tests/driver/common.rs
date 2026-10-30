use std::time::Duration;

use anyswap_sdk::{
    bitcoin::{
        hashes::{Hash as _, sha256},
        secp256k1::{Secp256k1, SecretKey},
    },
    driver::{
        ChainConfig, Decision, Endpoint, Height, Observed, Output, Sent, Spender, Status,
        SwapRecord, Timestamp, decide,
    },
    swap_script::{Side, SpendPath, Terms},
    types::Chain,
    utxo::SwapUtxo,
};
use lightning_invoice::{Currency, InvoiceBuilder, PaymentSecret};
use uuid::Uuid;

pub const STARTED_AT: Timestamp = 1_700_000_000;
pub const NOW: Timestamp = STARTED_AT + 60;
/// When a fixture swap with nothing on chain cancels: `started_at +
/// funding_timeout` under `ChainConfig::bitcoin()` on a SwapIn, and as its
/// hold invoice expires on a SwapOut.
pub const DEADLINE: Timestamp = STARTED_AT + 1_200;
pub const TIP: Height = 1_000;
pub const FEE_RATE: f64 = 2.0;
pub const SEND_AMOUNT: u64 = 101_000;
pub const RECEIVE_AMOUNT: u64 = 100_000;
pub const SPEND_FEE: u64 = 500;
pub const SWAP_IN_CSV: u32 = 144;
pub const SWAP_OUT_CSV: u32 = 288;
pub const PREIMAGE: [u8; 32] = [3; 32];

pub fn server_key() -> SecretKey {
    SecretKey::from_slice(&[7; 32]).unwrap()
}

pub fn txid(byte: u8) -> String {
    hex::encode([byte; 32])
}

pub fn opening_txid() -> String {
    txid(1)
}

pub fn spend_txid() -> String {
    txid(2)
}

fn terms(side: Side, csv: u32) -> Terms {
    Terms {
        swap_id: Uuid::from_u128(1),
        side,
        chain: Chain::Bitcoin,
        asset: "bitcoin".into(),
        csv,
        payment_hash: [9; 32],
        server_pubkey: server_key().public_key(&Secp256k1::new()),
        server_blinding_pubkey: None,
    }
}

fn record(source: Endpoint, dest: Endpoint) -> SwapRecord {
    SwapRecord {
        version: SwapRecord::VERSION,
        swap_id: Uuid::from_u128(1),
        payment_hash: [9; 32],
        source,
        dest,
        send_amount: SEND_AMOUNT,
        receive_amount: RECEIVE_AMOUNT,
        base_fee: SEND_AMOUNT - RECEIVE_AMOUNT,
        service_fee: 0,
        started_at: STARTED_AT,
        status: Status::Restoring,
        funds: Vec::new(),
        ended_at: None,
        preimage: None,
        sent: Sent::default(),
        server_status: None,
        server_cancel: None,
    }
}

pub fn swap_in() -> SwapRecord {
    let source = Endpoint::on_chain(terms(Side::Source, SWAP_IN_CSV), None);
    record(source, Endpoint::default())
}

pub fn swap_out() -> SwapRecord {
    let source = Endpoint::lightning(invoice(STARTED_AT, DEADLINE - STARTED_AT));
    let dest = Endpoint::on_chain(terms(Side::Dest, SWAP_OUT_CSV), Some("claim".into()));
    record(source, dest)
}

pub fn observed(outputs: Vec<Output>) -> Observed {
    Observed {
        now: NOW,
        tip: TIP,
        fee_rate: FEE_RATE,
        outputs,
        server: None,
    }
}

/// The height at which a transaction has `confs` confirmations at `TIP`.
pub fn confs(confs: u32) -> Option<Height> {
    Some(TIP + 1 - confs)
}

pub fn output(txid: String, value: u64, height: Option<Height>) -> Output {
    Output {
        utxo: SwapUtxo::bitcoin(txid.clone(), 0, value),
        value,
        height,
        spend: None,
    }
}

/// The SwapIn deposit or the SwapOut opening, whichever `record` expects.
pub fn opening(record: &SwapRecord, height: Option<Height>) -> Output {
    let value = match record.source.chain {
        Some(_) => record.send_amount,
        None => record.receive_amount,
    };
    output(opening_txid(), value, height)
}

pub fn spent(mut output: Output, path: SpendPath, height: Option<Height>) -> Output {
    output.spend = Some(Spender {
        txid: spend_txid(),
        height,
        path,
        fee: Some(SPEND_FEE),
    });
    output
}

pub fn run(record: &SwapRecord, observed: &Observed) -> Decision {
    run_with(record, observed, &ChainConfig::bitcoin())
}

pub fn run_with(record: &SwapRecord, observed: &Observed, chain_config: &ChainConfig) -> Decision {
    let side = match record.source.chain {
        Some(_) => Side::Source,
        None => Side::Dest,
    };
    let terms = record.terms(side).unwrap();
    decide(record, &terms, observed, chain_config)
}

/// A regtest invoice created at `created_at` that expires `expiry` later.
pub fn invoice(created_at: Timestamp, expiry: u64) -> String {
    let secp = Secp256k1::new();
    let key = SecretKey::from_slice(&[5; 32]).unwrap();
    InvoiceBuilder::new(Currency::Regtest)
        .description("test".into())
        .payment_hash(sha256::Hash::from_byte_array([9; 32]))
        .payment_secret(PaymentSecret([8; 32]))
        .duration_since_epoch(Duration::from_secs(created_at))
        .min_final_cltv_expiry_delta(144)
        .amount_milli_satoshis(SEND_AMOUNT * 1000)
        .expiry_time(Duration::from_secs(expiry))
        .build_signed(|hash| secp.sign_ecdsa_recoverable(hash, &key))
        .unwrap()
        .to_string()
}
