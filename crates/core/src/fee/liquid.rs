//! Liquid on-chain base-fee estimate.
//!
//! Liquid transaction sizes are substantially larger than Bitcoin's because
//! confidential outputs carry asset/value commitments and rangeproofs. The
//! size constants are owned by the Liquid transaction builders and this module
//! applies the same `fee_rate * vsize * safety` policy as Bitcoin.

use elements::Transaction;

use crate::{
    transaction::{
        TxError,
        liquid::{CLAIM_VSIZE, OPENING_VSIZE, REFUND_VSIZE},
    },
    types::SwapType,
};

/// Required fee for an already-built Liquid transaction at `fee_rate` sat/vB.
///
/// This is used by the Liquid transaction builders after inserting their
/// estimated and final witnesses.
///
/// Sized on the ELIP-0200 discounted vsize.
pub(crate) fn liquid_fee_for_tx(tx: &Transaction, fee_rate: f64) -> Result<u64, TxError> {
    if !fee_rate.is_finite() || fee_rate < 0.0 {
        return Err(TxError::InvalidFeeRate(fee_rate));
    }
    fee_for_vsize(tx.discount_vsize() as u64, fee_rate).ok_or(TxError::FeeOverflow)
}

/// Fee a Liquid transaction of `vsize` vbytes pays at `fee_rate` sat/vB. `vsize`
/// is the ELIP-0200 discounted one, which is what the builders charge against.
///
/// `None` if `fee_rate` is not a finite, non-negative number, or if the fee
/// overflows.
pub fn fee_for_vsize(vsize: u64, fee_rate: f64) -> Option<u64> {
    if !fee_rate.is_finite() || fee_rate < 0.0 {
        return None;
    }
    let fee = (vsize as f64 * fee_rate).ceil();
    (fee <= u64::MAX as f64).then_some(fee as u64)
}

/// Estimated Liquid base fee at `fee_rate` sat/vB, scaled by `safety`.
///
/// The estimate covers every on-chain transaction expected for the requested
/// swap path and saturates at `u64::MAX` on multiplication overflow.
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
    use elements::{LockTime, Transaction};

    use super::*;

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
        assert_eq!(
            base_fee(SwapType::SwapIn, 10.0, 1.5),
            base_fee(SwapType::SwapIn, 10.0, 1.0) * 3 / 2,
        );
    }

    #[test]
    fn base_fee_at_the_relay_minimum() {
        assert_eq!(base_fee(SwapType::SwapIn, 0.1, 1.0), 25);
        assert_eq!(base_fee(SwapType::SwapOut, 0.1, 1.0), 60);
    }

    #[test]
    fn base_fee_saturates_instead_of_overflowing() {
        assert_eq!(base_fee(SwapType::SwapIn, f64::MAX, 1.0), u64::MAX);
    }

    #[test]
    fn transaction_fee_reports_an_overflowing_rate() {
        let tx = Transaction {
            version: 2,
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![],
        };

        assert!(matches!(
            liquid_fee_for_tx(&tx, f64::MAX),
            Err(TxError::FeeOverflow)
        ));
    }

    #[test]
    fn transaction_fee_rejects_invalid_rate() {
        let tx = Transaction {
            version: 2,
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![],
        };

        assert!(matches!(
            liquid_fee_for_tx(&tx, -1.0),
            Err(TxError::InvalidFeeRate(rate)) if rate == -1.0
        ));
    }
}
