use std::time::Duration;

use anyswap_sdk::driver::{ChainConfig, Config, DriverError, Fund, FundState, Refusal, Status};
use tokio::time::Instant;

use crate::world::{Behavior, FEE_RATE, Reach, World, user_address};

#[tokio::test(start_paused = true)]
async fn completes_with_the_server_claim_as_payout() {
    let world = World::bitcoin();
    let swap = world.swap_in(100_000).await;
    let opening = world.deposit(&swap).await;
    world.chain.mine(3);
    world.server.pay_invoice(swap.swap_id());
    swap.step().await.unwrap();
    let claim = world.server.claim(swap.swap_id()).unwrap();
    world.chain.mine(1);

    let status = swap.step().await.unwrap();

    assert_eq!(
        status,
        Status::Completed {
            opening,
            payout: Some(claim),
            fee: None,
        }
    );
    assert!(world.server.refund_key(swap.swap_id()).is_some());
}

#[tokio::test(start_paused = true)]
async fn refunds_by_keypath_once_the_server_times_out() {
    let world = World::bitcoin();
    let swap = world.swap_in(100_000).await;
    world.deposit(&swap).await;
    world.chain.mine(1);
    world.server.time_out(swap.swap_id());
    swap.step().await.unwrap();
    let refund = swap.refund(&user_address()).await.unwrap().unwrap();
    world.chain.mine(1);

    swap.step().await.unwrap();

    let funds = swap.funds().await.unwrap();
    assert!(matches!(
        &funds[..],
        [Fund { state: FundState::Refunded { txid, .. }, .. }] if *txid == refund
    ));
}

#[tokio::test(start_paused = true)]
async fn refuses_a_refund_to_another_network() {
    let world = World::bitcoin();
    let swap = world.swap_in(100_000).await;
    world.deposit(&swap).await;
    world.chain.mine(1);
    world.server.time_out(swap.swap_id());
    swap.step().await.unwrap();

    let refused = swap
        .refund("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4")
        .await;

    let funds = swap.funds().await.unwrap();
    assert!(matches!(
        refused,
        Err(DriverError::Refused(Refusal::Address(_)))
    ));
    assert_eq!(funds[0].state, FundState::Refundable);
}

#[tokio::test(start_paused = true)]
async fn evicted_refund_is_refundable_again() {
    let world = World::bitcoin();
    let swap = world.swap_in(100_000).await;
    world.deposit(&swap).await;
    world.chain.mine(1);
    world.server.time_out(swap.swap_id());
    swap.step().await.unwrap();
    let refund = swap.refund(&user_address()).await.unwrap().unwrap();
    world.chain.evict(&refund);

    swap.step().await.unwrap();

    let funds = swap.funds().await.unwrap();
    assert_eq!(funds[0].state, FundState::Refundable);
}

#[tokio::test(start_paused = true)]
async fn replaces_a_pending_refund_once_fees_rise() {
    let world = World::bitcoin();
    let swap = world.swap_in(100_000).await;
    world.deposit(&swap).await;
    world.chain.mine(1);
    world.server.time_out(swap.swap_id());
    swap.step().await.unwrap();
    let refund = swap.refund(&user_address()).await.unwrap().unwrap();
    world.chain.set_fee_rate(30.0);

    let replacement = swap.refund(&user_address()).await.unwrap();

    let funds = swap.funds().await.unwrap();
    assert!(replacement.as_ref().is_some_and(|txid| *txid != refund));
    assert!(matches!(
        &funds[..],
        [Fund { state: FundState::Refunding { txid, .. }, .. }] if Some(txid) == replacement.as_ref()
    ));
}

#[tokio::test(start_paused = true)]
async fn watches_a_locked_deposit_with_no_destination_until_it_is_refundable() {
    let world = World::bitcoin();
    let swap = world.swap_in(100_000).await;
    world.deposit(&swap).await;
    world.server.time_out(swap.swap_id());
    world.sweep().await;
    world
        .advance(world.config.refund_timeout + Duration::from_secs(1))
        .await;
    world.chain.mine(1);

    world.sweep().await;

    let funds = swap.funds().await.unwrap();
    assert_eq!(funds[0].state, FundState::Refundable);
}

#[tokio::test(start_paused = true)]
async fn shows_dust_with_no_destination_refundable_once_fees_fall() {
    let world = World::bitcoin();
    let swap = world.swap_in(100_000).await;
    world.server.time_out(swap.swap_id());
    world.chain.pay(&swap.address().await.unwrap(), 2_000);
    world.chain.mine(1);
    world.chain.set_fee_rate(20.0);
    world.sweep().await;
    world
        .advance(world.config.refund_timeout + Duration::from_secs(1))
        .await;
    world.chain.set_fee_rate(FEE_RATE);

    world.sweep().await;

    let funds = swap.funds().await.unwrap();
    assert_eq!(funds[0].state, FundState::Refundable);
}

#[tokio::test(start_paused = true)]
async fn shows_funding_timeout_when_the_invoice_outlives_the_server() {
    let world = World::bitcoin();
    let started = world.now();

    let swap = world.try_swap_in(100_000, 86_400).await.unwrap();

    assert_eq!(
        swap.status().await.unwrap(),
        Status::Funding {
            expires_at: started + 1_200,
            cancel_sent: false,
        }
    );
}

#[tokio::test(start_paused = true)]
async fn refuses_a_funding_timeout_the_server_does_not_wait_out() {
    let world = World::with(
        Config::default(),
        ChainConfig {
            funding_timeout: Duration::from_secs(7_200),
            ..ChainConfig::bitcoin()
        },
    );

    let refused = world.try_swap_in(100_000, 86_400).await;

    assert!(matches!(
        refused,
        Err(DriverError::Refused(Refusal::FundingTimeout { .. }))
    ));
}

#[tokio::test(start_paused = true)]
async fn brings_the_deadline_forward_for_an_invoice_shorter_than_the_funding_timeout() {
    let world = World::bitcoin();
    let started = world.now();

    let swap = world.try_swap_in(100_000, 3_000).await.unwrap();

    assert_eq!(
        swap.status().await.unwrap(),
        Status::Funding {
            expires_at: started + 600,
            cancel_sent: false,
        }
    );
}

#[tokio::test(start_paused = true)]
async fn refuses_an_invoice_that_ends_before_a_deposit_can_confirm() {
    let world = World::bitcoin();

    let refused = world.try_swap_in(100_000, 2_400).await;

    assert!(matches!(
        refused,
        Err(DriverError::Refused(Refusal::Invoice(_)))
    ));
}

#[tokio::test(start_paused = true)]
async fn cancels_at_its_deadline_on_the_driver_clock() {
    let world = World::bitcoin();
    let swap = world.swap_in(100_000).await;
    let Status::Funding { expires_at, .. } = swap.status().await.unwrap() else {
        panic!("a new swap is funding");
    };
    world
        .advance(Duration::from_secs(expires_at - world.now()))
        .await;
    world.sweep().await;

    world.sweep().await;

    let status = swap.status().await.unwrap();
    assert!(matches!(status, Status::Canceled { opening: None, .. }));
}

#[tokio::test(start_paused = true)]
async fn advance_waits_out_the_signal_gap_under_a_flood() {
    let world = World::bitcoin();
    let swap = world.swap_in(100_000).await;
    let flood = world.flood(swap.swap_id());
    let start = Instant::now();

    swap.advance().await.unwrap();

    flood.abort();
    let waited = start.elapsed();
    assert!(waited >= Duration::from_secs(5));
    assert!(waited < world.config.poll_interval);
}

#[tokio::test(start_paused = true)]
async fn shows_the_server_error_until_a_read_works() {
    let world = World::bitcoin();
    let swap = world.swap_in(100_000).await;
    world.deposit(&swap).await;
    world.server.behave(Behavior {
        reach: Reach::Down,
        ..Behavior::default()
    });
    swap.step().await.unwrap();
    let down = swap.server_error();
    world.server.behave(Behavior::default());

    swap.step().await.unwrap();

    assert!(down.is_some_and(|error| error.retryable()));
    assert!(swap.server_error().is_none());
}

#[tokio::test(start_paused = true)]
async fn step_fails_retryably_while_the_chain_is_down() {
    let world = World::bitcoin();
    let swap = world.swap_in(100_000).await;
    world.chain.down(true);

    let error = swap.step().await.unwrap_err();

    assert!(error.retryable());
}
