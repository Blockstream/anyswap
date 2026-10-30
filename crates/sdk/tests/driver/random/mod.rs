mod swap_in;
mod swap_out;

use std::{future::Future, time::Duration};

use anyswap_sdk::{
    driver::{ChainConfig, Config, Handle},
    wallet::Wallet,
};
use proptest::prelude::*;

use crate::world::{Reach, World};

/// A world whose CSVs and claim window are a few blocks long, so a random
/// sequence of moves reaches the end of a swap.
fn world() -> World {
    let config = Config {
        fee_rate_cache_ttl: Duration::ZERO,
        ..Config::default()
    };
    let chain_config = ChainConfig {
        min_confs: 2,
        claim_margin: 4,
        max_refund_csv: 8,
        ..ChainConfig::bitcoin()
    };
    World::with(config, chain_config)
}

/// Runs `test` on a fresh runtime with tokio's clock paused, as
/// `#[tokio::test(start_paused = true)]` does.
fn run(test: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap()
        .block_on(test);
}

/// A block is mined, a block time passes and the driver steps, as a running
/// driver does at least once a block.
async fn block(world: &World, swap: &Handle<Wallet>) {
    world.chain.mine(1);
    world
        .advance(Duration::from_secs(world.chain_config.block_time))
        .await;
    let _ = swap.step().await;
}

fn fee_rate() -> impl Strategy<Value = f64> {
    (1..200u32).prop_map(f64::from)
}

fn reach() -> impl Strategy<Value = Reach> {
    prop_oneof![Just(Reach::Up), Just(Reach::Down), Just(Reach::Hang)]
}
