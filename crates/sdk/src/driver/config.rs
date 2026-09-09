use std::time::Duration;

/// Knobs true of the driver whatever it is swapping. Wall-clock deadlines are
/// counted on the driver's own clock, never from a time the server reports.
#[derive(Clone, Debug)]
pub struct Config {
    /// From `started_at`: when a swap with nothing on chain closes.
    pub funding_timeout: Duration,
    /// From `committed_at`: how long a SwapOut waits for the server's key
    /// before claiming by preimage. Zero claims straight away.
    pub coop_timeout: Duration,
    /// The longest a loop waits between passes without a server notification.
    pub poll_interval: Duration,
    /// The target every fee estimate is read at, in blocks.
    pub fee_conf_target: u32,
    /// From entering `Refund`: how long one with nothing under its script
    /// stays in `run`'s set.
    pub refund_timeout: Duration,
    /// Lets `restore` take a swap the server holds no signed request for on
    /// the server's word, as a client from before the server kept one did.
    /// Off, such a swap is reported and left alone.
    pub trust_unsigned: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            funding_timeout: Duration::from_secs(1200),
            coop_timeout: Duration::from_secs(300),
            poll_interval: Duration::from_secs(15),
            fee_conf_target: 3,
            refund_timeout: Duration::from_secs(3600),
            trust_unsigned: false,
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
    /// How far past `dest_csv` a hold invoice may hold the user's payment.
    pub max_htlc_hold: u32,
    /// The rate no spend goes above, in sat/vB.
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
            max_htlc_hold: 504,
            max_fee_rate: 100.0,
            block_time: 600,
        }
    }

    pub fn liquid() -> Self {
        Self {
            min_confs: 2,
            claim_margin: 2520,
            max_refund_csv: 10080,
            max_htlc_hold: 5040,
            max_fee_rate: 1.0,
            block_time: 60,
        }
    }
}
