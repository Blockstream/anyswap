//! Swap pricing. [`swap_fee`] and [`service_fee`] are chain-neutral arithmetic;
//! [`base_fee`] is the on-chain estimate and dispatches by [`Chain`], since the
//! transactions a swap broadcasts differ per chain. The Bitcoin estimate is in
//! [`bitcoin`], with Liquid to follow in [`liquid`].

use crate::types::{Chain, SwapType};

pub mod bitcoin;
pub mod liquid;

/// The total swap fee: the base fee plus the proportional service fee on
/// `receive_amount`. Saturates at `u64::MAX`.
pub fn swap_fee(receive_amount: u64, service_ppm: u64, base_fee: u64) -> u64 {
    base_fee.saturating_add(service_fee(receive_amount, service_ppm))
}

/// The proportional service fee: `receive_amount * service_ppm / 1_000_000`. Saturates at
/// `u64::MAX`.
pub fn service_fee(receive_amount: u64, service_ppm: u64) -> u64 {
    (receive_amount as u128 * service_ppm as u128 / 1_000_000).min(u64::MAX as u128) as u64
}

/// Estimated on-chain base fee for the transactions a swap of `swap_type`
/// broadcasts on `chain`, at `fee_rate` sat/vB, scaled by `safety` (policy
/// multiplier, `1.0` = none).
///
/// `fee_rate` is sat/vB to match what `ChainClient::get_fee_rate` returns; each
/// chain converts it and owns its own saturation behaviour.
pub fn base_fee(chain: Chain, swap_type: SwapType, fee_rate: f64, safety: f64) -> u64 {
    match chain {
        Chain::Bitcoin => bitcoin::base_fee(swap_type, fee_rate, safety),
        Chain::Liquid => liquid::base_fee(swap_type, fee_rate, safety),
    }
}

/// Fee a transaction of `vsize` vbytes pays on `chain` at `fee_rate` sat/vB. The
/// single definition of what a chain charges, so quotes, the transaction
/// builders, and callers weighing an output all price the same way.
///
/// `None` if `fee_rate` is not a finite, non-negative number, or if the fee
/// overflows.
pub fn fee_for_vsize(chain: Chain, vsize: u64, fee_rate: f64) -> Option<u64> {
    match chain {
        Chain::Bitcoin => bitcoin::fee_for_vsize(vsize, fee_rate),
        Chain::Liquid => liquid::fee_for_vsize(vsize, fee_rate),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fee_for_vsize_rejects_a_bad_rate_the_same_way_on_every_chain() {
        for chain in [Chain::Bitcoin, Chain::Liquid] {
            for rate in [f64::NAN, -1.0, f64::INFINITY] {
                assert_eq!(fee_for_vsize(chain, 250, rate), None, "{chain:?} at {rate}");
            }
        }
    }

    #[test]
    fn service_fee_cases() {
        assert_eq!(service_fee(1_000_000, 0), 0);
        assert_eq!(service_fee(0, 10_000), 0);
        assert_eq!(service_fee(1_000_000, 1_000), 1_000);
        assert_eq!(service_fee(2_000_000, 500), 1_000);
        assert_eq!(service_fee(1, 1), 0);
        assert_eq!(service_fee(999_999, 1), 0);
        assert_eq!(service_fee(1_000_000, 1), 1);
        assert_eq!(service_fee(u64::MAX, u64::MAX), u64::MAX);
    }

    #[test]
    fn swap_fee_adds_base_fee() {
        // base_fee 500 plus service_fee(1_000_000, 1_000) = 1_000.
        assert_eq!(swap_fee(1_000_000, 1_000, 500), 1_500);
        assert_eq!(swap_fee(0, 0, 0), 0);
        assert_eq!(swap_fee(1_000, 0, u64::MAX), u64::MAX);
    }

    #[test]
    fn base_fee_dispatches_to_the_chain() {
        assert_eq!(
            base_fee(Chain::Bitcoin, SwapType::SwapIn, 10.0, 1.0),
            bitcoin::base_fee(SwapType::SwapIn, 10.0, 1.0),
        );
        assert_eq!(
            base_fee(Chain::Liquid, SwapType::SwapOut, 0.1, 1.0),
            liquid::base_fee(SwapType::SwapOut, 0.1, 1.0),
        );
    }
}
