use std::{collections::HashMap, sync::Mutex, time::Duration};

use anyswap_sdk::{
    bitcoin::{
        hashes::{Hash as _, sha256},
        secp256k1::{Secp256k1, SecretKey},
    },
    driver::Timestamp,
};
use lightning_invoice::{Currency, InvoiceBuilder, PaymentSecret};

/// The user's Lightning node: the claim invoices it mints and the preimages
/// behind them, which the server learns by paying one.
#[derive(Default)]
pub struct Lightning {
    preimages: Mutex<HashMap<[u8; 32], [u8; 32]>>,
}

impl Lightning {
    pub fn invoice(&self, amount: u64, created_at: Timestamp, expiry: u64) -> String {
        let mut preimages = self.preimages.lock().unwrap();
        let count = preimages.len() as u64;
        let preimage = sha256::Hash::hash(&count.to_be_bytes()).to_byte_array();
        let payment_hash = sha256::Hash::hash(&preimage).to_byte_array();
        preimages.insert(payment_hash, preimage);
        bolt11(payment_hash, amount, created_at, expiry, [6; 32])
    }

    pub fn preimage(&self, payment_hash: &[u8; 32]) -> Option<[u8; 32]> {
        self.preimages.lock().unwrap().get(payment_hash).copied()
    }
}

/// A regtest invoice for `amount` sats signed by the node key `node`.
pub fn bolt11(
    payment_hash: [u8; 32],
    amount: u64,
    created_at: Timestamp,
    expiry: u64,
    node: [u8; 32],
) -> String {
    let key = SecretKey::from_slice(&node).unwrap();
    InvoiceBuilder::new(Currency::Regtest)
        .description("anyswap".into())
        .payment_hash(sha256::Hash::from_byte_array(payment_hash))
        .payment_secret(PaymentSecret(payment_hash))
        .duration_since_epoch(Duration::from_secs(created_at))
        .min_final_cltv_expiry_delta(144)
        .amount_milli_satoshis(amount * 1000)
        .expiry_time(Duration::from_secs(expiry))
        .build_signed(|hash| Secp256k1::new().sign_ecdsa_recoverable(hash, &key))
        .unwrap()
        .to_string()
}
