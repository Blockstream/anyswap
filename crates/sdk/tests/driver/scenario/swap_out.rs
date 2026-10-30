use std::time::Duration;

use anyswap_sdk::driver::{CancelReason, ChainConfig, Config, DriverError, Refusal, Status};

use crate::world::{Behavior, Reach, World};

#[tokio::test(start_paused = true)]
async fn refuses_a_hold_invoice_that_outlasts_funding_timeout() {
    let world = World::with(
        Config::default(),
        ChainConfig {
            funding_timeout: Duration::from_secs(300),
            ..ChainConfig::bitcoin()
        },
    );

    let refused = world.try_swap_out(100_000).await;

    assert!(matches!(
        refused,
        Err(DriverError::Refused(Refusal::Invoice(_)))
    ));
}

#[tokio::test(start_paused = true)]
async fn claims_by_keypath_once_the_server_settles() {
    let world = World::bitcoin();
    let swap = world.swap_out(100_000).await;
    world.pay(&swap);
    world.server.open(swap.swap_id());
    world.chain.mine(3);
    swap.step().await.unwrap();
    let Status::Claiming {
        claim: Some(claim), ..
    } = swap.step().await.unwrap()
    else {
        panic!("the claim goes out once the server's key is in");
    };

    let status = world.finish(&swap).await;

    assert!(world.server.settled(swap.swap_id()));
    assert!(matches!(
        status,
        Status::Completed { payout: Some(payout), .. } if payout == claim
    ));
}

#[tokio::test(start_paused = true)]
async fn claims_in_the_sweep_after_the_reveal_without_waiting_to_poll() {
    let world = World::bitcoin();
    world.server.behave(Behavior {
        withhold_key: true,
        ..Behavior::default()
    });
    let swap = world.swap_out(100_000).await;
    world.pay(&swap);
    world.server.open(swap.swap_id());
    world.chain.mine(3);

    world.sweep().await;

    assert!(matches!(
        swap.status().await.unwrap(),
        Status::Claiming { claim: Some(_), .. }
    ));
}

#[tokio::test(start_paused = true)]
async fn claims_by_preimage_while_the_server_withholds_its_key() {
    let world = World::bitcoin();
    world.server.behave(Behavior {
        withhold_key: true,
        ..Behavior::default()
    });
    let swap = world.swap_out(100_000).await;
    world.pay(&swap);
    world.server.open(swap.swap_id());
    world.chain.mine(3);
    swap.step().await.unwrap();

    let status = swap.step().await.unwrap();

    assert!(world.server.settled(swap.swap_id()));
    assert!(matches!(status, Status::Claiming { claim: Some(_), .. }));
}

#[tokio::test(start_paused = true)]
async fn claims_by_preimage_while_the_server_is_down() {
    let world = World::bitcoin();
    let swap = world.swap_out(100_000).await;
    world.pay(&swap);
    world.server.open(swap.swap_id());
    world.chain.mine(3);
    world.server.behave(Behavior {
        reach: Reach::Down,
        ..Behavior::default()
    });
    swap.step().await.unwrap();

    let status = swap.step().await.unwrap();

    assert!(matches!(status, Status::Claiming { claim: Some(_), .. }));
}

#[tokio::test(start_paused = true)]
async fn claims_by_preimage_past_a_hanging_server() {
    let world = World::bitcoin();
    let swap = world.swap_out(100_000).await;
    world.pay(&swap);
    world.server.open(swap.swap_id());
    world.chain.mine(3);
    world.server.behave(Behavior {
        reach: Reach::Hang,
        ..Behavior::default()
    });
    swap.step().await.unwrap();

    let status = swap.step().await.unwrap();

    assert!(matches!(status, Status::Claiming { claim: Some(_), .. }));
}

#[tokio::test(start_paused = true)]
async fn ends_refunded_while_a_fee_spike_holds_the_preimage() {
    let world = World::bitcoin();
    let swap = world.swap_out(100_000).await;
    world.pay(&swap);
    let opening = world.server.open(swap.swap_id()).unwrap();
    world.chain.set_fee_rate(150.0);
    world.chain.mine(3);
    swap.step().await.unwrap();
    world.chain.mine(world.server.dest_csv() - 3);
    world.server.refund(swap.swap_id()).unwrap();
    world.chain.mine(3);

    let status = swap.step().await.unwrap();

    assert!(!world.server.settled(swap.swap_id()));
    assert_eq!(
        status,
        Status::Canceled {
            opening: Some(opening),
            reason: CancelReason::ServerRefunded,
        }
    );
}

#[tokio::test(start_paused = true)]
async fn holds_the_preimage_while_its_claim_cannot_pay_the_estimate() {
    let world = World::bitcoin();
    let swap = world.swap_out(10_000).await;
    world.pay(&swap);
    world.server.open(swap.swap_id());
    world.chain.set_fee_rate(80.0);
    world.chain.mine(3);

    let status = swap.step().await.unwrap();

    assert!(!world.server.settled(swap.swap_id()));
    assert!(matches!(
        status,
        Status::Confirming {
            fee_rate_spike: Some(80.0),
            ..
        }
    ));
}

#[tokio::test(start_paused = true)]
async fn claims_on_regtest_while_the_chain_has_no_fee_estimate() {
    let world = World::bitcoin();
    let swap = world.swap_out(100_000).await;
    world.pay(&swap);
    world.server.open(swap.swap_id());
    world.chain.estimates(false);

    let status = world.finish(&swap).await;

    assert!(world.server.settled(swap.swap_id()));
    assert!(matches!(status, Status::Completed { .. }));
}

#[tokio::test(start_paused = true)]
async fn claims_within_what_a_small_opening_can_pay() {
    let world = World::bitcoin();
    let swap = world.swap_out(5_000).await;
    world.pay(&swap);
    world.server.open(swap.swap_id());
    world.chain.mine(3);
    swap.step().await.unwrap();
    world.chain.set_fee_rate(48.0);

    let status = world.finish(&swap).await;

    assert!(world.server.settled(swap.swap_id()));
    assert!(matches!(status, Status::Completed { .. }));
}

#[tokio::test(start_paused = true)]
async fn restored_after_a_lost_store_claims_by_keypath() {
    let mut world = World::bitcoin();
    let swap = world.swap_out(100_000).await;
    let swap_id = swap.swap_id();
    world.pay(&swap);
    world.server.open(swap_id);
    world.chain.mine(3);
    swap.step().await.unwrap();
    world.lose_store();
    let swap = world.restore(swap_id).await;

    let status = swap.step().await.unwrap();

    assert!(matches!(status, Status::Claiming { claim: Some(_), .. }));
}

#[tokio::test(start_paused = true)]
async fn restored_without_the_servers_key_outbids_its_keypath_claim() {
    let mut world = World::bitcoin();
    let swap = world.swap_out(100_000).await;
    let swap_id = swap.swap_id();
    world.pay(&swap);
    world.server.open(swap_id);
    world.chain.mine(3);
    swap.step().await.unwrap();
    let Status::Claiming {
        claim: Some(keypath),
        ..
    } = swap.step().await.unwrap()
    else {
        panic!("the claim goes out once the server's key is in");
    };
    world.lose_store();
    world.server.behave(Behavior {
        withhold_key: true,
        hide_claim: true,
        ..Behavior::default()
    });
    world.chain.set_fee_rate(2.0 * world.chain.fee_rate());
    let swap = world.restore(swap_id).await;

    let status = swap.step().await.unwrap();

    assert!(matches!(
        status,
        Status::Claiming { claim: Some(claim), .. } if claim != keypath
    ));
}

#[tokio::test(start_paused = true)]
async fn claim_reorged_out_stays_claiming() {
    let world = World::bitcoin();
    let swap = world.swap_out(100_000).await;
    world.pay(&swap);
    world.server.open(swap.swap_id());
    world.chain.mine(3);
    swap.step().await.unwrap();
    let Status::Claiming {
        claim: Some(claim), ..
    } = swap.step().await.unwrap()
    else {
        panic!("the claim goes out once the server's key is in");
    };
    world.chain.mine(1);
    world.chain.reorg(1);

    let status = swap.step().await.unwrap();

    assert!(matches!(
        status,
        Status::Claiming { claim: Some(pending), confs: 0, .. } if pending == claim
    ));
}
