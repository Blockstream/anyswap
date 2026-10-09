// What the page implements for the driver, and what crosses it as a plain
// object. Pasted verbatim into the generated `.d.ts`: declarations only.

/**
 * The server API as the driver calls it, over the wire objects as they are.
 * Amounts in a body are decimal strings, as on the wire; every integer
 * is a number. A throw with a numeric `status` is the server's
 * answer, its `code` the server's error code; any other throw is the server
 * being unreachable.
 */
export interface ServerClient {
  getInfo(): Promise<unknown>;
  createSwap(body: unknown): Promise<void>;
  getSwap(swapId: string): Promise<unknown>;
  listSwaps(query: unknown): Promise<unknown>;
  cancel(swapId: string, body: unknown): Promise<void>;
  revealClaim(swapId: string, body: unknown): Promise<void>;
  revealPreimage(swapId: string, body: unknown): Promise<void>;
}

/** One transaction of an address's history; `height` is absent in the mempool. */
export interface HistoryTx {
  hex: string;
  height?: number | null;
}

/** Query and broadcast on one chain. A throw fails the pass, and the next pass asks again. */
export interface ChainClient {
  getHeight(): Promise<number>;
  /** Estimate for confirmation within `target` blocks, in sat/vB. */
  getFeeRate(target: number): Promise<number>;
  /** Every transaction paying or spending `address`, mempool included. */
  addressHistory(address: string): Promise<HistoryTx[]>;
  /**
   * The transaction spending output `vout` of `txid`, mempool included, or
   * `undefined` while it is unspent: Esplora's `/tx/:txid/outspend/:vout`
   * with the spending transaction fetched. Read from the output itself, so it
   * is found however many other transactions crowd the address's history.
   */
  outspend(txid: string, vout: number): Promise<HistoryTx | undefined>;
  /** Broadcasts a signed transaction and returns its txid. */
  broadcast(txHex: string): Promise<string>;
}

/**
 * Durable swap records, keyed by their `swap_id`, kept as the plain objects
 * given. Amounts in one are decimal strings and every integer a number,
 * and `load` hands them back the same way. `list` gives every stored
 * `swap_id` without reading the records, so one that fails to load stops
 * only its own swap.
 */
export interface SwapStore {
  save(record: unknown): Promise<void>;
  load(swapId: string): Promise<unknown | undefined>;
  list(): Promise<string[]>;
}

/** A push channel naming swaps worth reading sooner; resolves `undefined` while it is down. */
export interface Signals {
  next(): Promise<string | undefined>;
}

/**
 * Where funds land when a swap is created without an address. A throw or a
 * rejection fails whatever asked for one. Must resolve promptly: the loop
 * ticks swaps one at a time and waits on it.
 */
export interface Destinations {
  /** Where a swap-out on `chain` claims to; asked at creation. */
  claimAddress(chain: string): string | Promise<string>;
  /** Where a swap-in on `chain` refunds to; asked with something spendable under the script. */
  refundAddress(chain: string): string | Promise<string>;
}

/** A quote: one read of the server's terms, locked by `quote_id` until `expires_at`. Amounts in sats, as decimal strings. */
export interface Quote {
  source_chain: string | null;
  source_asset_id: string | null;
  dest_chain: string | null;
  dest_asset_id: string | null;
  receive_amount: string;
  send_amount: string;
  base_fee: string;
  /** `send_amount - receive_amount - base_fee`. */
  service_fee: string;
  /** The whole spread, `send_amount - receive_amount`. */
  fee: string;
  quote_id: string;
  /** Unix seconds. */
  expires_at: number;
}

/** The end a quote is pinned by: the amount to receive, or the amount to send, in sats. */
export type TargetAmount = { receive: bigint } | { send: bigint };
