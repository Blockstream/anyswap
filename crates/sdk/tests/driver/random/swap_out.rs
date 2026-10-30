use anyswap_sdk::driver::Status;
use proptest::prelude::*;

use super::{block, fee_rate, reach, run, world};
use crate::world::{Behavior, Reach};

#[derive(Clone, Debug)]
enum Move {
    Step,
    Block,
    FeeRate(f64),
    Reorg(u32),
    Open,
    Refund,
    TimeOut,
    WithholdKey(bool),
    Reach(Reach),
}

fn moves() -> impl Strategy<Value = Move> {
    prop_oneof![
        Just(Move::Step),
        Just(Move::Block),
        fee_rate().prop_map(Move::FeeRate),
        (1..=2u32).prop_map(Move::Reorg),
        Just(Move::Open),
        Just(Move::Refund),
        Just(Move::TimeOut),
        any::<bool>().prop_map(Move::WithholdKey),
        reach().prop_map(Move::Reach),
    ]
}

proptest! {
    #[test]
    fn never_loses_the_payment(
        amount in prop_oneof![5_000..20_000u64, 5_000..500_000u64],
        moves in prop::collection::vec(moves(), 1..40),
    ) {
        run(async {
            let world = world();
            let swap = world.swap_out(amount).await;
            let id = swap.swap_id();
            world.pay(&swap);
            let mut behavior = Behavior::default();
            for m in moves {
                match m {
                    Move::Step => {
                        let _ = swap.step().await;
                    }
                    Move::Block => block(&world, &swap).await,
                    Move::FeeRate(rate) => world.chain.set_fee_rate(rate),
                    Move::Reorg(depth) => world.chain.reorg(depth),
                    Move::Open => {
                        world.server.open(id);
                    }
                    Move::Refund => {
                        world.server.refund(id);
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

            let status = world.finish(&swap).await;

            assert!(
                !world.server.settled(id) || matches!(status, Status::Completed { .. }),
                "the server took the payment and the swap ended {status:?}"
            );
        });
    }
}
