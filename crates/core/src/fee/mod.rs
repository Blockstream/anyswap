//! Swap pricing. The amount arithmetic is chain-neutral; on-chain fees
//! dispatch by [`Chain`], since the transactions a swap broadcasts differ per
//! chain.

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

/// The least the user must send to receive `receive_amount`.
pub fn min_send_for_receive(receive_amount: u64, service_ppm: u64, base_fee: u64) -> Option<u64> {
    receive_amount.checked_add(swap_fee(receive_amount, service_ppm, base_fee))
}

/// The most the user can receive for `send_amount`.
pub fn max_receive_for_send(send_amount: u64, service_ppm: u64, base_fee: u64) -> Option<u64> {
    let net = send_amount.checked_sub(base_fee)? as u128;
    // The largest `r` with `r + r * service_ppm / 1_000_000 <= net`, the fee floored.
    Some((((net + 1) * 1_000_000 - 1) / (1_000_000 + service_ppm as u128)) as u64)
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
    fn max_receive_for_send_is_the_largest_that_fits() {
        for (send, ppm, base) in [
            (1_000_000, 0, 0),
            (253_000, 4_000, 2_000),
            (253_001, 4_000, 2_000),
            (2_250, 4_000, 2_000),
            (1, 10_000, 0),
            (2_000, 1_000, 2_000),
            (13_171_977_363_711, 123_456, 0),
        ] {
            let receive = max_receive_for_send(send, ppm, base).expect("covers the base fee");
            let spent = receive + swap_fee(receive, ppm, base);
            assert!(spent <= send, "{send}/{ppm}/{base} spends {spent}");
            assert!(
                receive + 1 + swap_fee(receive + 1, ppm, base) > send,
                "{send}/{ppm}/{base} leaves room for one more"
            );
            assert!(
                send - spent <= 1,
                "{send}/{ppm}/{base} leaves {}",
                send - spent
            );
        }
    }

    #[test]
    fn max_receive_for_send_inverts_the_worked_example() {
        assert_eq!(swap_fee(250_000, 4_000, 2_000), 3_000);
        assert_eq!(min_send_for_receive(250_000, 4_000, 2_000), Some(253_000));
        assert_eq!(max_receive_for_send(253_000, 4_000, 2_000), Some(250_000));
        assert_eq!(max_receive_for_send(2_000, 4_000, 2_000), Some(0));
        assert_eq!(max_receive_for_send(1_999, 4_000, 2_000), None);
    }

    #[test]
    fn min_send_for_receive_overflows_to_none() {
        assert_eq!(min_send_for_receive(u64::MAX, 0, 1), None);
        assert_eq!(min_send_for_receive(1, 1_000_000, u64::MAX), None);
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
