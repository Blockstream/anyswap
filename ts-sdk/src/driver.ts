/**
 * The driver: runs swaps to completion against the chain, in the SDK's wasm,
 * over a server client, a chain client and a store the application supplies.
 *
 * It creates swaps from a quote, watches the swap script's address, sends the
 * protocol messages when their moment comes, claims a swap-out and refunds a
 * swap-in through the cheapest path available, and saves every change to your
 * store, so a restart resumes where it left off. It trusts only what it can
 * verify: the terms are checked against your own request at creation and
 * frozen, and from each later read it takes the server's key and preimage on
 * their own merit, and its status label and cancel reason for display only.
 *
 * Two ways to drive. {@link Driver.run} drives every stored swap in the
 * background and {@link Driver.subscribe} yields each change; a
 * {@link SwapHandle} drives one swap by hand with {@link SwapHandle.advance}.
 *
 * What you supply is three interfaces, two of which the SDK's own classes
 * satisfy as they are: {@link Api!AnySwapClient | AnySwapClient} is a
 * {@link ServerClient}, {@link Esplora!BitcoinEsplora | BitcoinEsplora} and
 * {@link Esplora!LiquidEsplora | LiquidEsplora} are {@link ChainClient}s, and
 * the {@link SwapStore} is yours, with {@link MemoryStore} for tests.
 * {@link Api!AnySwapClient.signals | AnySwapClient.signals} is the optional
 * {@link Signals} channel that paces the loops, and {@link Destinations} the
 * optional hook for a wallet that mints its addresses on demand.
 *
 * @packageDocumentation
 * @module Driver
 */

import type {
  ChainClient,
  Driver as WasmDriver,
  DriverError as WasmDriverError,
  DriverErrorCode,
  Quote,
  RunState,
  ServerClient,
  Signals,
  SwapHandle as WasmSwapHandle,
  TargetAmount,
  Updates as WasmUpdates,
} from "@blockstream/anyswap-wasm";
import type { SwapContext } from "./context";
import {
  Chain,
  type OutPoint,
  type SwapEndpoint,
  type SwapErrorCode,
  type SwapType,
} from "./types";
import { getWasm } from "./wasm";

export type {
  ChainClient,
  DriverErrorCode,
  Quote,
  RunState,
  ServerClient,
  Signals,
  TargetAmount,
};

/** Unix seconds. */
export type Timestamp = number;
/** A block height on the swap's chain. */
export type Height = number;

/**
 * Where a swap stands, resolved from the chain on every pass. `status` names
 * the variant and its fields sit beside it, so `status.status === "Canceled"`
 * narrows. Deadlines are absolute, so two quiet passes produce an equal
 * status. `expires_at` is the time to pay by: a swap-in's is `minConfs + 1`
 * blocks before its invoice expires, a swap-out's no later than the expiry of
 * its claim invoice. `Restoring` is no pass's answer: {@link Driver.restore}
 * sets it and the first pass replaces it. `fee` is what a swap-out's claim pays, in sats
 * as a decimal string, read from the transaction on chain; `null` on a
 * swap-in's `Completed`, whose payout is the server's. `Confirming` carries
 * the chain's `maxFeeRate` as `max_fee_rate`, and a swap-out's carries
 * `fee_rate_spike`, the fee estimate in sat/vB, while it holds the preimage:
 * above that, or more than a claim of the funds can pay. It leaves it out
 * otherwise.
 */
export type DriverStatus =
  | { status: "Restoring" }
  | { status: "Funding"; expires_at: Timestamp; cancel_sent: boolean }
  | {
      status: "Confirming";
      txid: string;
      confs: number;
      needed: number;
      max_fee_rate: number;
      fee_rate_spike?: number;
    }
  | {
      status: "Claiming";
      opening: string;
      claim: string | null;
      confs: number;
      fee: string | null;
    }
  | {
      status: "Completed";
      opening: string;
      payout: string | null;
      fee: string | null;
    }
  | { status: "Canceled"; opening: string | null; reason: CancelReason };

/**
 * The server's cancel in its words: its {@link Types!SwapErrorCode | error
 * code} and the message the cancel came with. Shown, never verified.
 */
export interface ServerCancel {
  error_type: SwapErrorCode | null;
  cancel_message: string | null;
}

/**
 * Why a swap ended `Canceled`, named by `kind`. `ServerCanceled`: the server
 * gave the swap up, with its {@link ServerCancel} words beside. `CsvMatured`:
 * a swap-in's deposit matured its CSV with no word from the server.
 * `ServerRefunded`: the server's CSV refund took a swap-out's opening.
 * `ClaimWindowClosed`: less than `claimMargin` of a swap-out's CSV was left
 * before its preimage went out, so the payment fails back once the server's
 * CSV refund takes the opening, from block `at`.
 */
export type CancelReason =
  | ({ kind: "ServerCanceled" } & ServerCancel)
  | { kind: "CsvMatured" }
  | { kind: "ServerRefunded" }
  | { kind: "ClaimWindowClosed"; at: Height };

/**
 * Where the refund of one output of the user's stands, named by `state`.
 * `Locked` is under the CSV until block `at`, or unconfirmed (`null`);
 * `Refundable` is spendable and worth its refund, so the next refund takes it;
 * `Dust` is spendable but not worth its fee plus the smallest refund output
 * at the last rate, and `run` makes it `Refundable` once the rate falls far enough;
 * `Refunding` and `Refunded` are in our refund `txid`, out of
 * a block and in one. Their `fee` is the whole refund transaction's, in sats
 * as a decimal string, so every fund it took shows the same one: group by
 * `txid` before adding fees up.
 */
export type FundState =
  | { state: "Locked"; at: Height | null }
  | { state: "Refundable" }
  | { state: "Dust" }
  | { state: "Refunding"; txid: string; fee: string | null }
  | { state: "Refunded"; txid: string; fee: string | null };

/**
 * Swap-in: one output under the script that is the user's. The deposit the
 * server was paid for is not one, nor is an output the server took by its
 * own path.
 */
export interface Fund {
  outpoint: OutPoint;
  /** In sats, as a decimal string. */
  amount: string;
  state: FundState;
}

/**
 * One end of a swap: on chain when `chain` is set, Lightning when it is not.
 * Keys are hex strings.
 */
export interface Endpoint {
  chain: Chain | null;
  asset: string | null;
  csv: number | null;
  server_pubkey: string | null;
  server_blinding_pubkey: string | null;
  /** On chain, where the end pays out: the refund address of a source, the claim address of a dest. */
  destination: string | null;
  /** Lightning: the invoice, paid to a swap-in, paid by a swap-out. */
  invoice: string | null;
  /** The server's key, once it arrived and verified against `server_pubkey`. */
  server_privkey: string | null;
}

/** The messages already delivered, since a message leaves no trace on chain. */
export interface Sent {
  cancel: boolean;
  refund_key: boolean;
  preimage: boolean;
}

/**
 * One swap as the driver keeps it: both ends frozen at acceptance, the status,
 * and what the chain cannot report. The JSON form of the Rust record, and the
 * document a {@link SwapStore} holds. Amounts are sats as decimal strings.
 */
export interface SwapRecord {
  version: number;
  swap_id: string;
  payment_hash: number[];
  source: Endpoint;
  dest: Endpoint;
  send_amount: string;
  receive_amount: string;
  /** The server's split of `send_amount - receive_amount`: `base_fee` for its transactions, `service_fee` the rest. */
  base_fee: string;
  service_fee: string;
  /** The driver's own clock at acceptance. */
  started_at: Timestamp;
  status: DriverStatus;
  /** Swap-in: what under the script is the user's, as the last pass left it. */
  funds: Fund[];
  /** When the status first became final: the driver's clock, or the server's on restore. */
  ended_at: Timestamp | null;
  /**
   * The preimage once seen and hashed to `payment_hash`: a swap-in's from the
   * server, a swap-out's own from a claim of ours on chain.
   */
  preimage: number[] | null;
  sent: Sent;
  /** The server's status label as the last pass that reached it read it. Shown, never decided on. */
  server_status: string | null;
  /** The server's cancel, kept from the first pass that read one. */
  server_cancel: ServerCancel | null;
}

/**
 * Durable swap records, wanted by id and nothing else. Back it with anything
 * that outlives the page, keeping each record as the object given: a future
 * SDK can upgrade old records only if they kept their self-describing form.
 * Must resolve promptly: the driver waits on each call while it holds the
 * swap's lock.
 */
export interface SwapStore {
  /** Replaces any record already stored under the same `swap_id`. */
  save(record: SwapRecord): Promise<void>;
  load(swapId: string): Promise<SwapRecord | undefined>;
  /**
   * Every stored `swap_id`, without reading the records, so one that fails
   * to load stops only its own swap.
   */
  list(): Promise<string[]>;
}

/** Records kept in memory, for tests and short-lived tools. */
export class MemoryStore implements SwapStore {
  private readonly records = new Map<string, SwapRecord>();

  async save(record: SwapRecord): Promise<void> {
    this.records.set(record.swap_id, record);
  }

  async load(swapId: string): Promise<SwapRecord | undefined> {
    return this.records.get(swapId);
  }

  async list(): Promise<string[]> {
    return [...this.records.keys()];
  }
}

/**
 * Where funds land when a swap is created without an address, for a wallet
 * that mints one on demand. A throw or a rejection leaves no address and fails
 * whatever asked for one. Resolve promptly: the loop ticks swaps one at a time
 * and waits on it.
 */
export interface Destinations {
  /** Where a swap-out on `chain` claims to; asked at creation. */
  claimAddress(chain: Chain): string | Promise<string>;
  /**
   * Where a swap-in on `chain` refunds to; asked once it has something
   * refundable under the script, and kept as its refund address once a refund
   * to it is out. An address of another network, or on Liquid one that is not
   * confidential, fails the refund, and the next pass asks again.
   */
  refundAddress(chain: Chain): string | Promise<string>;
}

/**
 * The knobs true of the driver whatever it swaps, for
 * {@link Driver.withConfig}. Each one left out keeps its default. Times are
 * seconds on the driver's own clock, never one the server reports.
 */
export interface DriverConfig {
  /** The longest wait between passes without a signal. Defaults to 15. */
  pollInterval?: number;
  /** How long a call to the server may go unanswered before it fails as `Transport`. Defaults to 30. */
  serverTimeout?: number;
  /** The target every fee estimate is read at, in blocks. Defaults to 3. */
  feeConfTarget?: number;
  /** How long a chain's fee estimate is reused before it is read again; zero reads it every pass. Defaults to 30. */
  feeRateCacheTtl?: number;
  /** From a swap-in's end, how long it keeps getting passes once nothing of the user's is left to move. Defaults to 3600. */
  refundTimeout?: number;
  /** The longest a swap-out's hold invoice may hold the payment, in Lightning blocks. Defaults to 1008. */
  maxHtlcHold?: number;
}

/**
 * The knobs that mean something per chain, for {@link Driver.withChain}. Each
 * one left out keeps the chain's default. Depths are in blocks, counted from
 * a confirmation, since a timelock on an unconfirmed output has not started.
 */
export interface ChainConfig {
  /**
   * The depth that releases a swap-out's preimage, and that a spend reaches
   * before it closes a swap, at least 1. Defaults to 3 on Bitcoin, 2 on
   * Liquid.
   */
  minConfs?: number;
  /** How much CSV must remain when a swap-out's preimage goes out. Defaults to 252 on Bitcoin, 2520 on Liquid. */
  claimMargin?: number;
  /** The longest CSV accepted on a swap-in deposit. Defaults to 1008 on Bitcoin, 10080 on Liquid. */
  maxRefundCsv?: number;
  /**
   * The rate no spend goes above, in sat/vB, finite and positive. A swap-out
   * holds its preimage while the estimate is above it. Defaults to 100 on
   * Bitcoin, 1 on Liquid.
   */
  maxFeeRate?: number;
  /**
   * Expected block interval in seconds, to compare a block deadline with a
   * wall-clock one. Defaults to 600 on Bitcoin, 60 on Liquid.
   */
  blockTime?: number;
  /**
   * From creation, the most a swap with nothing on chain is given, in seconds.
   * A swap-in asks the server to cancel at it, or at its claim invoice's
   * expiry if sooner, and is refused unless the server waits this long and
   * `minConfs + 1` blocks more and its claim invoice outlasts those blocks. A
   * swap-out is refused when its hold invoice outlasts it. Defaults to 1200 on
   * Bitcoin, 900 on Liquid.
   */
  fundingTimeout?: number;
}

/**
 * The answer to {@link SwapHandle.cancel}: `Requested` means the swap is
 * canceled or closes when the server confirms it, and `TooLate` means the
 * payment landed first and the swap continues.
 */
export type Cancellation = "Requested" | "TooLate";

/**
 * What a driver call failed with. `code` names the kind of failure, keeping
 * the server's own code names for its answers; `retryable` means the chain or
 * server was unreachable and the next call sees the same world, where anything
 * else needs attention before acting again.
 */
export class DriverError extends Error {
  constructor(
    message: string,
    public readonly code: DriverErrorCode,
    public readonly retryable: boolean,
    /** The HTTP status the server answered with, `undefined` when the failure is not an answer from the server. */
    public readonly httpStatus?: number,
  ) {
    super(message);
    this.name = "DriverError";
  }
}

/** What {@link Driver.restoreAll} found among the swaps the server lists and the store lacks. */
export interface RestoreReport {
  /** The ids of the swaps now in the store. */
  restored: string[];
  /** Each swap that failed a check, with the check; `Unsigned` for one the server holds no signed request for. */
  failed: { swapId: string; error: DriverError }[];
}

function fromWasm(e: WasmDriverError): DriverError {
  return new DriverError(e.message, e.code, e.retryable, e.httpStatus);
}

function convert(e: unknown): unknown {
  return e instanceof getWasm().DriverError ? fromWasm(e) : e;
}

async function call<T>(f: () => Promise<T>): Promise<T> {
  try {
    return await f();
  } catch (e) {
    throw convert(e);
  }
}

function sync<T>(f: () => T): T {
  try {
    return f();
  } catch (e) {
    throw convert(e);
  }
}

function chainOf(end: SwapEndpoint): string | undefined {
  return end === "lightning" ? undefined : end.chain;
}

function wasmConfig(config: DriverConfig) {
  const wasm = new (getWasm().Config)();
  const {
    pollInterval,
    serverTimeout,
    feeConfTarget,
    feeRateCacheTtl,
    refundTimeout,
    maxHtlcHold,
  } = config;
  if (pollInterval !== undefined) wasm.pollInterval = BigInt(pollInterval);
  if (serverTimeout !== undefined) wasm.serverTimeout = BigInt(serverTimeout);
  if (feeConfTarget !== undefined) wasm.feeConfTarget = feeConfTarget;
  if (feeRateCacheTtl !== undefined)
    wasm.feeRateCacheTtl = BigInt(feeRateCacheTtl);
  if (refundTimeout !== undefined) wasm.refundTimeout = BigInt(refundTimeout);
  if (maxHtlcHold !== undefined) wasm.maxHtlcHold = maxHtlcHold;
  return wasm;
}

function wasmChainConfig(chain: Chain, config: ChainConfig) {
  const { ChainConfig } = getWasm();
  const wasm =
    chain === Chain.Liquid ? ChainConfig.liquid() : ChainConfig.bitcoin();
  const {
    minConfs,
    claimMargin,
    maxRefundCsv,
    maxFeeRate,
    blockTime,
    fundingTimeout,
  } = config;
  if (minConfs !== undefined) wasm.minConfs = minConfs;
  if (claimMargin !== undefined) wasm.claimMargin = claimMargin;
  if (maxRefundCsv !== undefined) wasm.maxRefundCsv = maxRefundCsv;
  if (maxFeeRate !== undefined) wasm.maxFeeRate = maxFeeRate;
  if (blockTime !== undefined) wasm.blockTime = BigInt(blockTime);
  if (fundingTimeout !== undefined)
    wasm.fundingTimeout = BigInt(fundingTimeout);
  return wasm;
}

/**
 * Drives every swap in its store. Build one at startup, register a chain
 * client per chain, then either spawn {@link Driver.run} and watch
 * {@link Driver.subscribe}, or drive one swap at a time through its
 * {@link SwapHandle}.
 */
export class Driver {
  private inner: WasmDriver;

  /**
   * Throws an `Invalid` {@link DriverError} outside a secure context: a page
   * served over plain HTTP has no Web Locks, so tabs over one store could not
   * take turns on a swap.
   *
   * @param ctx The wallet and chain parameters every swap shares.
   * @param server The server API; an {@link Api!AnySwapClient | AnySwapClient} with auth.
   * @param store Where records live between passes and across restarts.
   */
  constructor(ctx: SwapContext, server: ServerClient, store: SwapStore) {
    this.inner = sync(() => new (getWasm().Driver)(ctx.inner, server, store));
  }

  /**
   * Registers a chain with its client, under the chain's default policy with
   * `config` applied over it; a chain without one cannot be swapped on.
   * Throws an `Invalid` {@link DriverError} for a `minConfs` of 0, or a
   * `maxFeeRate` that is not finite and positive.
   */
  withChain(chain: Chain, client: ChainClient, config?: ChainConfig): this {
    this.inner = sync(() =>
      this.inner.withChain(
        chain,
        client,
        config && wasmChainConfig(chain, config),
      ),
    );
    return this;
  }

  /** Sets the driver-wide knobs, each one left out at its default. */
  withConfig(config: DriverConfig): this {
    this.inner = this.inner.withConfig(wasmConfig(config));
    return this;
  }

  /**
   * Takes a push channel to pace the loops by, typically
   * {@link Api!AnySwapClient.signals | AnySwapClient.signals}. Without one they
   * pace on the poll interval alone.
   */
  withSignals(signals: Signals): this {
    this.inner = this.inner.withSignals(signals);
    return this;
  }

  /**
   * Takes the hook asked for an address when a swap is created without one:
   * {@link Driver.swapOut} with no `claimAddress`, and a swap-in with no
   * refund address once there is something to take.
   */
  withDestinations(destinations: Destinations): this {
    this.inner = this.inner.withDestinations(destinations);
    return this;
  }

  /** Wakes any loop waiting on `swapId`, as a signal from the channel would. */
  signal(swapId: string): void {
    sync(() => this.inner.signal(swapId));
  }

  /**
   * A quote for a swap from `source` to `dest`: one read of the server's
   * terms for one amount, with the `quote_id` that locks its base fee until
   * `expires_at`. `target` pins either end, `{ receive }` or `{ send }`. Show
   * it, create the swap when the user accepts.
   */
  quote(
    source: SwapEndpoint,
    dest: SwapEndpoint,
    target: TargetAmount,
  ): Promise<Quote> {
    return call(() => this.inner.quote(chainOf(source), chainOf(dest), target));
  }

  /**
   * Creates a swap-in against `quote` with the invoice minted for its
   * `receive_amount`. The handle carries where and how much to deposit. A
   * `refundAddress` is signed with the request and paid by any refund; without
   * one the {@link Destinations} hook is asked when there is something to take,
   * or {@link SwapHandle.refund} is called with an address. Refused unless the
   * server waits out the chain's `fundingTimeout` and `minConfs + 1` blocks
   * more, and the invoice outlasts those blocks.
   */
  swapIn(
    quote: Quote,
    claimInvoice: string,
    refundAddress?: string,
  ): Promise<SwapHandle> {
    return call(async () =>
      wrap(await this.inner.swapIn(quote, claimInvoice, refundAddress)),
    );
  }

  /**
   * Creates a swap-out against `quote` paying `claimAddress`, or the address
   * {@link Destinations} gives without one. The handle carries the hold invoice
   * to pay. Refused, and canceled at the server, when the hold invoice outlasts
   * the chain's `fundingTimeout`.
   */
  swapOut(quote: Quote, claimAddress?: string): Promise<SwapHandle> {
    return call(async () =>
      wrap(await this.inner.swapOut(quote, claimAddress)),
    );
  }

  /** The stored swap as a fresh handle, or `undefined`. */
  get(swapId: string): Promise<SwapHandle | undefined> {
    return call(async () => {
      const swap = await this.inner.get(swapId);
      return swap && wrap(swap);
    });
  }

  /**
   * Rebuilds one swap from the server's copy of the signed request that
   * created it, verified against this wallet's identity key, for a store that
   * lost it. It is `Restoring` until its first pass, which {@link Driver.run}
   * makes, or {@link SwapHandle.step}. A record already stored is returned as
   * is.
   */
  restore(swapId: string): Promise<SwapHandle> {
    return call(async () => wrap(await this.inner.restore(swapId)));
  }

  /**
   * Rebuilds every swap the server lists for this wallet that the store
   * lacks, as {@link Driver.restore} does one, and reports each swap restored
   * or the check that failed it.
   */
  restoreAll(): Promise<RestoreReport> {
    return call(async () => {
      const report = await this.inner.restoreAll();
      return {
        restored: report.restored,
        failed: report.failed.map((failure) => ({
          swapId: failure.swapId,
          error: fromWasm(failure.error),
        })),
      };
    });
  }

  /**
   * Drives every swap in the store that is not quiet, in the background,
   * until {@link Driver.stop}. Every pass re-reads the store, so a swap
   * created or restored is driven from the next pass on; a second call is a
   * no-op. Watch {@link Driver.subscribe} for what changed.
   */
  run(): void {
    this.inner.run();
  }

  /**
   * Ends {@link Driver.run} after the pass in flight. Final: a stopped driver
   * does not run again, and its {@link Updates} end once drained. Handles
   * still `step` and `advance`.
   */
  stop(): void {
    this.inner.stop();
  }

  /** Resolves once `run` has exited and no pass writes to the store. */
  waitStopped(): Promise<void> {
    return this.inner.waitStopped();
  }

  /** Where `run` is: `Idle` before it, then `Running`, `Stopping` and `Stopped`. */
  get state(): RunState {
    return this.inner.state;
  }

  /** Every swap a pass changed, from {@link Driver.run} and from {@link SwapHandle.step}. */
  subscribe(): Updates {
    return new Updates(this.inner.subscribe());
  }
}

function wrap(inner: WasmSwapHandle): SwapHandle {
  return new SwapHandle(inner);
}

/**
 * A swap bound to its driver: its id and type, and the calls that act on it.
 * It keeps no copy of the swap, so every read is the store's newest.
 */
export class SwapHandle {
  /** @internal */
  constructor(private readonly inner: WasmSwapHandle) {}

  get swapId(): string {
    return this.inner.swapId;
  }

  get swapType(): SwapType {
    return this.inner.swapType as SwapType;
  }

  /** The status as the last pass left it. */
  status(): Promise<DriverStatus> {
    return call(() => this.inner.status() as Promise<DriverStatus>);
  }

  /**
   * `Completed` or `Canceled`: the deal is settled. A swap-in's
   * {@link SwapHandle.funds} may still be moving.
   */
  isTerminal(): Promise<boolean> {
    return call(() => this.inner.isTerminal());
  }

  /** The record as the last pass left it. */
  record(): Promise<SwapRecord> {
    return call(() => this.inner.record() as Promise<SwapRecord>);
  }

  /** Swap-in: what under the script is the user's, as the last pass left it. */
  funds(): Promise<Fund[]> {
    return call(() => this.inner.funds() as Promise<Fund[]>);
  }

  /**
   * The error of this swap's last failing pass under {@link Driver.run},
   * `undefined` after a working one.
   */
  get error(): DriverError | undefined {
    const error = this.inner.error;
    return error && fromWasm(error);
  }

  /**
   * The error of the last pass that could not read this swap at the server,
   * which leaves the record's `server_status` as an earlier read left it;
   * `undefined` once a read works or the swap has ended.
   */
  get serverError(): DriverError | undefined {
    const error = this.inner.serverError;
    return error && fromWasm(error);
  }

  /** Swap-in: the deposit address; the record's `send_amount` goes here. */
  address(): Promise<string> {
    return call(() => this.inner.address());
  }

  /** Swap-out: the hold invoice to pay; the record's `send_amount` is its amount. */
  claimInvoice(): Promise<string> {
    return call(() => this.inner.claimInvoice());
  }

  /** One pass, now. Resolves to the status it left. */
  step(): Promise<DriverStatus> {
    return call(() => this.inner.step() as Promise<DriverStatus>);
  }

  /**
   * Waits for a signal, though at least 5 s, or the poll interval, whichever
   * comes first, then makes one pass. Resolves to the status it left.
   */
  advance(): Promise<DriverStatus> {
    return call(() => this.inner.advance() as Promise<DriverStatus>);
  }

  cancel(): Promise<Cancellation> {
    return call(() => this.inner.cancel() as Promise<Cancellation>);
  }

  /**
   * Swap-in: one pass that refunds the `Refundable` funds to `destination`,
   * in any status. An earlier refund still `Refunding` is replaced once the
   * fee estimate is above its rate. The refund's txid, or `undefined` with
   * nothing refundable yet or the earlier refund standing. `destination` must
   * be an address of the swap's network, confidential on Liquid, or the refund
   * is refused.
   */
  refund(destination: string): Promise<string | undefined> {
    return call(() => this.inner.refund(destination));
  }
}

/**
 * Every swap a pass changed, as {@link Driver.run} and {@link SwapHandle.step}
 * produce them, plus a handle for every failing pass under `run`. Iterate it
 * with `for await`.
 */
export class Updates implements AsyncIterable<SwapHandle> {
  /** @internal */
  constructor(private readonly inner: WasmUpdates) {}

  /** The next changed swap, or `undefined` once the driver is stopped and drained, or gone. */
  async next(): Promise<SwapHandle | undefined> {
    const swap = await this.inner.next();
    return swap && wrap(swap);
  }

  async *[Symbol.asyncIterator](): AsyncIterator<SwapHandle> {
    for (let swap = await this.next(); swap; swap = await this.next()) {
      yield swap;
    }
  }
}
