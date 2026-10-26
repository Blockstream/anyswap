/**
 * Client-wide setup: the wallet and the chain parameters the SDK needs.
 *
 * Build one {@link SwapContext} at startup and pass it to every swap method that
 * touches keys or the chain. The swap already knows its own chain, so the caller
 * never chooses anything per call; the context supplies what a swap cannot know
 * by itself.
 *
 * Liquid swaps additionally need the network's genesis block hash and its policy
 * asset (L-BTC). Read both from your own chain source, never from the swap
 * server: the genesis hash is committed in every taproot signature the wallet
 * produces, and the policy asset decides what a transaction pays its fee in, so a
 * server that supplied them could make the user sign transactions that never
 * confirm. A context built without them serves Bitcoin swaps only; Liquid methods
 * on it throw.
 *
 * @packageDocumentation
 * @module Context
 */

import type { SwapContext as WasmSwapContext } from "@blockstream/anyswap-wasm";
import type { Wallet } from "./wallet";
import { getWasm } from "./wasm";

/**
 * Liquid chain parameters. Both come from your own chain source, never from the
 * swap server - see the {@link "Context" | module docs}.
 */
export interface LiquidParams {
  /** Genesis block hash of the Liquid network, hex-encoded. */
  genesisHash: string;
  /** Policy (fee) asset id of the Liquid network, hex-encoded. L-BTC. */
  policyAsset: string;
}

/** The wallet and chain parameters shared by every swap of one client. */
export class SwapContext {
  /** Underlying wasm context. Exposed so swap helpers can forward it. */
  readonly inner: WasmSwapContext;

  /**
   * @param wallet Wallet whose keys sign this client's swaps.
   * @param liquid Liquid chain parameters. Omit for a Bitcoin-only client.
   */
  constructor(wallet: Wallet, liquid?: LiquidParams) {
    this.inner = new (getWasm().SwapContext)(
      wallet.inner,
      liquid?.genesisHash,
      liquid?.policyAsset,
    );
  }

  /** Network this client operates on, from the wallet. */
  get network(): string {
    return this.inner.network;
  }
}
