use std::{str::FromStr, sync::Mutex};

use anyswap_sdk::{
    bitcoin::{
        Address, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness,
        absolute,
        hashes::{Hash as _, sha256},
        relative, transaction,
    },
    driver::{ChainClient, ChainError, Height, HistoryTx},
    transaction::bitcoin::{decode_tx_hex, encode_tx_hex},
};
use async_trait::async_trait;

/// Bitcoin in memory: blocks over a starting tip, a mempool, and a fee
/// estimate. A broadcast is checked for what the driver's decisions turn on:
/// its inputs exist and are unspent or replaceable by a higher fee rate, its
/// CSV has matured, its outputs are above dust. Signatures are not checked.
pub struct FakeChain(Mutex<State>);

struct State {
    base: Height,
    blocks: Vec<Vec<Transaction>>,
    mempool: Vec<Transaction>,
    fee_rate: f64,
    estimates: bool,
    down: bool,
    coins: u64,
}

impl FakeChain {
    pub fn new(tip: Height, fee_rate: f64) -> Self {
        Self(Mutex::new(State {
            base: tip,
            blocks: Vec::new(),
            mempool: Vec::new(),
            fee_rate,
            estimates: true,
            down: false,
            coins: 0,
        }))
    }

    /// Mines the mempool into the next block, then `blocks - 1` empty ones.
    pub fn mine(&self, blocks: u32) {
        let mut state = self.0.lock().unwrap();
        for _ in 0..blocks {
            let block = std::mem::take(&mut state.mempool);
            state.blocks.push(block);
        }
    }

    /// Drops the last `depth` blocks, their transactions back in the mempool.
    pub fn reorg(&self, depth: u32) {
        let mut state = self.0.lock().unwrap();
        let keep = state.blocks.len().saturating_sub(depth as usize);
        let mut mempool: Vec<_> = state.blocks.drain(keep..).flatten().collect();
        mempool.append(&mut state.mempool);
        state.mempool = mempool;
    }

    /// Drops `txid` and what spends it from the mempool.
    pub fn evict(&self, txid: &str) {
        let txid = Txid::from_str(txid).unwrap();
        self.0.lock().unwrap().evict(txid);
    }

    pub fn set_fee_rate(&self, sat_per_vb: f64) {
        self.0.lock().unwrap().fee_rate = sat_per_vb;
    }

    pub fn fee_rate(&self) -> f64 {
        self.0.lock().unwrap().fee_rate
    }

    /// While off, fee reads fail, as on an indexer with no estimates.
    pub fn estimates(&self, on: bool) {
        self.0.lock().unwrap().estimates = on;
    }

    /// While down, every call the driver makes fails.
    pub fn down(&self, down: bool) {
        self.0.lock().unwrap().down = down;
    }

    /// Pays `amount` to `address` into the mempool, from a coin no swap
    /// knows, and returns the txid. The output is at vout 0.
    pub fn pay(&self, address: &str, amount: u64) -> String {
        let mut state = self.0.lock().unwrap();
        state.coins += 1;
        let coin = sha256::Hash::hash(&state.coins.to_be_bytes()).to_byte_array();
        let tx = Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::new(Txid::from_byte_array(coin), 0),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(amount),
                script_pubkey: script_pubkey(address),
            }],
        };
        let txid = tx.compute_txid();
        state.mempool.push(tx);
        txid.to_string()
    }

    /// The oldest output paying exactly `amount` to `address`.
    pub fn find(&self, address: &str, amount: u64) -> Option<(OutPoint, Amount)> {
        let spk = script_pubkey(address);
        let state = self.0.lock().unwrap();
        state.txs().find_map(|(tx, _)| {
            let vout = tx
                .output
                .iter()
                .position(|o| o.script_pubkey == spk && o.value.to_sat() == amount)?;
            Some((
                OutPoint::new(tx.compute_txid(), vout as u32),
                tx.output[vout].value,
            ))
        })
    }

    /// How many blocks `txid` is in, 0 in the mempool or unknown.
    pub fn confs(&self, txid: &Txid) -> u32 {
        let state = self.0.lock().unwrap();
        let height = state
            .txs()
            .find(|(tx, _)| tx.compute_txid() == *txid)
            .and_then(|(_, height)| height);
        height.map_or(0, |height| state.tip() + 1 - height)
    }

    /// Whether anything spends `outpoint`, in a block or the mempool.
    pub fn spent(&self, outpoint: &OutPoint) -> bool {
        self.0.lock().unwrap().spender(outpoint).is_some()
    }

    /// Whether a transaction in a block spends output `vout` of `txid` and
    /// pays `address`.
    pub fn spent_to(&self, txid: &str, vout: u32, address: &str) -> bool {
        let outpoint = OutPoint::new(Txid::from_str(txid).unwrap(), vout);
        let spk = script_pubkey(address);
        let state = self.0.lock().unwrap();
        state.spender(&outpoint).is_some_and(|(tx, height)| {
            height.is_some() && tx.output.iter().any(|o| o.script_pubkey == spk)
        })
    }

    /// Takes `tx` into the mempool under the checks a broadcast gets.
    pub fn submit(&self, tx: Transaction) -> Result<String, ChainError> {
        self.0
            .lock()
            .unwrap()
            .accept(tx)
            .map(|txid| txid.to_string())
    }

    fn reachable(&self) -> Result<std::sync::MutexGuard<'_, State>, ChainError> {
        let state = self.0.lock().unwrap();
        match state.down {
            true => Err(ChainError("down".into())),
            false => Ok(state),
        }
    }
}

#[async_trait]
impl ChainClient for FakeChain {
    async fn height(&self) -> Result<Height, ChainError> {
        Ok(self.reachable()?.tip())
    }

    async fn fee_rate(&self, _target: u32) -> Result<f64, ChainError> {
        let state = self.reachable()?;
        match state.estimates {
            true => Ok(state.fee_rate),
            false => Err(ChainError("no fee estimate".into())),
        }
    }

    async fn address_history(&self, address: &str) -> Result<Vec<HistoryTx>, ChainError> {
        let state = self.reachable()?;
        let spk = script_pubkey(address);
        let history = state
            .txs()
            .filter(|(tx, _)| {
                let pays = tx.output.iter().any(|o| o.script_pubkey == spk);
                let spends = tx.input.iter().any(|i| {
                    state
                        .output(&i.previous_output)
                        .is_some_and(|(o, _)| o.script_pubkey == spk)
                });
                pays || spends
            })
            .map(|(tx, height)| HistoryTx {
                hex: encode_tx_hex(tx),
                height,
            })
            .collect();
        Ok(history)
    }

    async fn outspend(&self, txid: &str, vout: u32) -> Result<Option<HistoryTx>, ChainError> {
        let state = self.reachable()?;
        let outpoint = OutPoint::new(Txid::from_str(txid).unwrap(), vout);
        Ok(state.spender(&outpoint).map(|(tx, height)| HistoryTx {
            hex: encode_tx_hex(tx),
            height,
        }))
    }

    async fn broadcast(&self, tx_hex: &str) -> Result<String, ChainError> {
        let tx = decode_tx_hex(tx_hex).map_err(|e| ChainError(e.to_string()))?;
        self.reachable()?.accept(tx).map(|txid| txid.to_string())
    }
}

impl State {
    fn tip(&self) -> Height {
        self.base + self.blocks.len() as Height
    }

    fn txs(&self) -> impl Iterator<Item = (&Transaction, Option<Height>)> {
        let confirmed = self.blocks.iter().enumerate().flat_map(|(i, block)| {
            let height = self.base + 1 + i as Height;
            block.iter().map(move |tx| (tx, Some(height)))
        });
        confirmed.chain(self.mempool.iter().map(|tx| (tx, None)))
    }

    fn output(&self, outpoint: &OutPoint) -> Option<(&TxOut, Option<Height>)> {
        self.txs()
            .find(|(tx, _)| tx.compute_txid() == outpoint.txid)
            .and_then(|(tx, height)| Some((tx.output.get(outpoint.vout as usize)?, height)))
    }

    fn spender(&self, outpoint: &OutPoint) -> Option<(&Transaction, Option<Height>)> {
        self.txs()
            .find(|(tx, _)| tx.input.iter().any(|i| i.previous_output == *outpoint))
    }

    /// What `tx` pays its miner over its inputs, `None` while one is unknown.
    fn fee(&self, tx: &Transaction) -> Option<u64> {
        let input = tx.input.iter().try_fold(0, |total, i| {
            Some(total + self.output(&i.previous_output)?.0.value.to_sat())
        })?;
        let output: u64 = tx.output.iter().map(|o| o.value.to_sat()).sum();
        input.checked_sub(output)
    }

    fn accept(&mut self, tx: Transaction) -> Result<Txid, ChainError> {
        let txid = tx.compute_txid();
        if self.txs().any(|(known, _)| known.compute_txid() == txid) {
            return Ok(txid);
        }
        let refuse = |why: &str| Err(ChainError(format!("{txid}: {why}")));
        let mut conflicts = Vec::new();
        for input in &tx.input {
            let Some((_, height)) = self.output(&input.previous_output) else {
                return refuse("missing input");
            };
            let confs = height.map_or(0, |height| self.tip() + 1 - height);
            match input.sequence.to_relative_lock_time() {
                Some(relative::LockTime::Blocks(csv)) if confs < u32::from(csv.value()) => {
                    return refuse("non-BIP68-final");
                }
                Some(relative::LockTime::Time(_)) => return refuse("time lock unsupported"),
                _ => {}
            }
            match self.spender(&input.previous_output) {
                Some((_, Some(_))) => return refuse("input spent in a block"),
                Some((other, None)) => conflicts.push(other.compute_txid()),
                None => {}
            }
        }
        if tx
            .output
            .iter()
            .any(|o| o.value < o.script_pubkey.minimal_non_dust())
        {
            return refuse("dust");
        }
        let Some(fee) = self.fee(&tx) else {
            return refuse("outputs exceed inputs");
        };
        let rate = fee as f64 / tx.vsize() as f64;
        for conflict in &conflicts {
            let old = self
                .mempool
                .iter()
                .find(|tx| tx.compute_txid() == *conflict);
            let (old_fee, old_vsize) = old
                .and_then(|old| Some((self.fee(old)?, old.vsize())))
                .unwrap_or((u64::MAX, 1));
            if fee <= old_fee || rate <= old_fee as f64 / old_vsize as f64 {
                return refuse("insufficient fee to replace");
            }
        }
        for conflict in conflicts {
            self.evict(conflict);
        }
        self.mempool.push(tx);
        Ok(txid)
    }

    fn evict(&mut self, txid: Txid) {
        let Some(index) = self.mempool.iter().position(|tx| tx.compute_txid() == txid) else {
            return;
        };
        self.mempool.remove(index);
        let children: Vec<_> = self
            .mempool
            .iter()
            .filter(|tx| tx.input.iter().any(|i| i.previous_output.txid == txid))
            .map(Transaction::compute_txid)
            .collect();
        for child in children {
            self.evict(child);
        }
    }
}

fn script_pubkey(address: &str) -> ScriptBuf {
    Address::from_str(address)
        .unwrap()
        .assume_checked()
        .script_pubkey()
}
