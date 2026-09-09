//! Bitcoin on-chain base-fee estimate: `fee_rate * vsize` of the transactions a
//! swap broadcasts. The vsizes are pinned against actually-built transactions by
//! `transaction::bitcoin`'s `swap_tx_vsizes` test.

use crate::{
    transaction::{CLAIM_VSIZE, OPENING_VSIZE, REFUND_VSIZE, bitcoin::sat_per_vb},
    types::SwapType,
};

/// Fee a Bitcoin transaction of `vsize` vbytes pays at `fee_rate` sat/vB. Goes
/// through [`sat_per_vb`], so callers price at the rate the builders will pay.
///
/// `None` if `fee_rate` is not a finite, non-negative number, or if the fee
/// overflows.
pub fn fee_for_vsize(vsize: u64, fee_rate: f64) -> Option<u64> {
    sat_per_vb(fee_rate)
        .ok()
        .and_then(|rate| rate.fee_vb(vsize))
        .map(|amount| amount.to_sat())
}

/// Estimated on-chain base fee at `fee_rate` sat/vB, scaled by `safety` (policy
/// multiplier, `1.0` = none). Saturates at `u64::MAX` rather than overflowing.
pub fn base_fee(swap_type: SwapType, fee_rate: f64, safety: f64) -> u64 {
    let vsize = match swap_type {
        SwapType::SwapIn => CLAIM_VSIZE,
        SwapType::SwapOut => OPENING_VSIZE + REFUND_VSIZE,
        SwapType::SwapChain => OPENING_VSIZE + REFUND_VSIZE + CLAIM_VSIZE,
    };
    let fee = fee_for_vsize(vsize, fee_rate).unwrap_or(u64::MAX);
    ((fee as f64) * safety).ceil() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::KEYPATH_VSIZE;

    #[test]
    fn fee_for_vsize_prices_a_single_input_refund() {
        assert_eq!(fee_for_vsize(KEYPATH_VSIZE, 10.0), Some(1_110));
        assert_eq!(fee_for_vsize(REFUND_VSIZE, 10.0), Some(1_380));
    }

    #[test]
    fn fee_for_vsize_keeps_a_fractional_rate() {
        assert_eq!(fee_for_vsize(REFUND_VSIZE, 1.5), Some(207));
        assert_eq!(fee_for_vsize(CLAIM_VSIZE, 1.069), Some(163));
    }

    #[test]
    fn base_fee_by_swap_type() {
        assert_eq!(base_fee(SwapType::SwapIn, 10.0, 1.0), 10 * CLAIM_VSIZE);
        assert_eq!(
            base_fee(SwapType::SwapOut, 10.0, 1.0),
            10 * (OPENING_VSIZE + REFUND_VSIZE)
        );
        assert_eq!(
            base_fee(SwapType::SwapChain, 10.0, 1.0),
            10 * (OPENING_VSIZE + REFUND_VSIZE + CLAIM_VSIZE)
        );
        // safety multiplier rounds up: 10 * 152 * 1.5 = 2280.
        assert_eq!(base_fee(SwapType::SwapIn, 10.0, 1.5), 2280);
    }

    #[test]
    fn base_fee_saturates_instead_of_overflowing() {
        assert_eq!(base_fee(SwapType::SwapIn, f64::MAX, 1.0), u64::MAX);
    }
}
