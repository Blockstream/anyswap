use std::time::Duration;

/// Knobs true of the driver whatever it is swapping. Wall-clock deadlines are
/// counted on the driver's own clock, never from a time the server reports.
#[derive(Clone, Debug)]
pub struct Config {
    /// From `started_at`: the latest a swap with nothing on chain closes. Its
    /// invoice's expiry, or the server's cancel, ends it sooner.
    pub funding_timeout: Duration,
    /// The longest a loop waits between passes without a server notification.
    pub poll_interval: Duration,
    /// How long a call to the server may go unanswered before it fails as a
    /// transport error.
    pub server_timeout: Duration,
    /// The target every fee estimate is read at, in blocks.
    pub fee_conf_target: u32,
    /// How long a chain's fee estimate is reused before it is read again.
    /// Zero reads it on every pass.
    pub fee_rate_cache_ttl: Duration,
    /// From a SwapIn's end: how long it stays in `run`'s set once nothing of
    /// ours under its script is left to move, so a deposit that crossed the
    /// end is found without a manual look.
    pub refund_timeout: Duration,
    /// The longest a SwapOut's hold invoice may hold the user's payment, in
    /// Lightning blocks: the most its `min_final_cltv_expiry_delta` may be.
    pub max_htlc_hold: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            funding_timeout: Duration::from_secs(86_400),
            poll_interval: Duration::from_secs(15),
            server_timeout: Duration::from_secs(30),
            fee_conf_target: 3,
            fee_rate_cache_ttl: Duration::from_secs(30),
            refund_timeout: Duration::from_secs(3600),
            max_htlc_hold: 1008,
        }
    }
}

/// Knobs that only mean anything per chain, passed with the chain in
/// [`Driver::with_chain`](super::Driver::with_chain). Block deadlines count
/// from a confirmation height, since a timelock on an unconfirmed output has
/// not started.
#[derive(Clone, Copy, Debug)]
pub struct ChainConfig {
    /// The depth that opens the SwapOut release gate, and the depth a spend
    /// reaches before it closes a swap.
    pub min_confs: u32,
    /// How much CSV must remain when the preimage goes out.
    pub claim_margin: u32,
    /// The longest CSV accepted on a deposit of our own.
    pub max_refund_csv: u32,
    /// The rate no spend goes above, in sat/vB. A SwapOut holds its preimage
    /// while the estimate is above it.
    pub max_fee_rate: f64,
    /// Expected block interval in seconds, to compare a block deadline against
    /// a wall-clock one.
    pub block_time: u64,
}

impl ChainConfig {
    pub fn bitcoin() -> Self {
        Self {
            min_confs: 3,
            claim_margin: 252,
            max_refund_csv: 1008,
            max_fee_rate: 100.0,
            block_time: 600,
        }
    }

    pub fn liquid() -> Self {
        Self {
            min_confs: 2,
            claim_margin: 2520,
            max_refund_csv: 10080,
            max_fee_rate: 1.0,
            block_time: 60,
        }
    }
}
