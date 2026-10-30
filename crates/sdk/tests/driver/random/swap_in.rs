use std::time::Duration;

use proptest::prelude::*;

use super::{block, fee_rate, reach, run, world};
use crate::world::{Behavior, FEE_RATE, Reach, user_address};

#[derive(Clone, Debug)]
enum Move {
    Step,
    Block,
    Sweep,
    FeeRate(f64),
    Reorg(u32),
    Deposit,
    DepositOther(u64),
    Refund,
    PayInvoice,
    Claim,
    TimeOut,
    WithholdKey(bool),
    Reach(Reach),
}

fn moves() -> impl Strategy<Value = Move> {
    prop_oneof![
        Just(Move::Step),
        Just(Move::Block),
        Just(Move::Sweep),
        fee_rate().prop_map(Move::FeeRate),
        (1..=2u32).prop_map(Move::Reorg),
        Just(Move::Deposit),
        (1_000..200_000u64).prop_map(Move::DepositOther),
        Just(Move::Refund),
        Just(Move::PayInvoice),
        Just(Move::Claim),
        Just(Move::TimeOut),
        any::<bool>().prop_map(Move::WithholdKey),
        reach().prop_map(Move::Reach),
    ]
}

proptest! {
    #[test]
    fn unpaid_deposits_come_back(
        amount in 5_000..500_000u64,
        moves in prop::collection::vec(moves(), 1..40),
    ) {
        run(async {
            let world = world();
            let swap = world.swap_in(amount).await;
            let id = swap.swap_id();
            let address = swap.address().await.unwrap();
            let mut deposits = Vec::new();
            let mut behavior = Behavior::default();
            for m in moves {
                match m {
                    Move::Step => {
                        let _ = swap.step().await;
                    }
                    Move::Block => block(&world, &swap).await,
                    Move::Sweep => world.sweep().await,
                    Move::FeeRate(rate) => world.chain.set_fee_rate(rate),
                    Move::Reorg(depth) => world.chain.reorg(depth),
                    Move::Deposit => deposits.push(world.deposit(&swap).await),
                    Move::DepositOther(value) => deposits.push(world.chain.pay(&address, value)),
                    Move::Refund => {
                        let _ = swap.refund(&user_address()).await;
                    }
                    Move::PayInvoice => world.server.pay_invoice(id),
                    Move::Claim => {
                        world.server.claim(id);
                    }
                    Move::TimeOut => world.server.time_out(id),
                    Move::WithholdKey(withhold) => {
                        behavior.withhold_key = withhold;
                        world.server.behave(behavior);
                    }
                    Move::Reach(reach) => {
                        behavior.reach = reach;
                        world.server.behave(behavior);
                    }
                }
            }

            world.chain.set_fee_rate(FEE_RATE);
            let refunded = || {
                deposits
                    .iter()
                    .all(|txid| world.chain.spent_to(txid, 0, &user_address()))
            };
            let block_time = Duration::from_secs(world.chain_config.block_time);
            for _ in 0..4 * world.server.source_csv() {
                if refunded() {
                    break;
                }
                let _ = swap.refund(&user_address()).await;
                world.chain.mine(1);
                world.advance(block_time).await;
            }

            let funds = swap.funds().await.unwrap();
            assert!(
                world.server.paid(id) || refunded(),
                "the server never paid and a deposit stayed: {funds:?}"
            );
        });
    }
}
