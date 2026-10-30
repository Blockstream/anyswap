use super::{
    record::Timestamp,
    rt::{MaybeSend, MaybeSync},
};

/// The wall clock the driver counts its deadlines on: `funding_timeout`,
/// `refund_timeout`, a quote's expiry and the fee estimate's cache. Block
/// deadlines are counted on the chain.
pub trait Clock: MaybeSend + MaybeSync {
    /// Seconds since the Unix epoch.
    fn now(&self) -> Timestamp;
}

/// The system's clock, the browser's on wasm32.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    #[allow(clippy::disallowed_methods)]
    fn now(&self) -> Timestamp {
        anyswap_core::utils::now()
    }
}
