/**
 * Esplora blockchain clients for fetching on-chain data needed during swap execution.
 *
 * The driver's `ChainClient` on each chain: it reads address histories and spends,
 * broadcasts the user's claim or refund, and estimates fee rates.
 *
 * One client per chain: {@link BitcoinEsplora} and {@link LiquidEsplora}. The two
 * chains need separate backends and floor their fee estimates at different relay
 * minimums.
 *
 * Optional: both implement {@link Esplora}, a structural interface, so anything
 * with those methods replaces them.
 *
 * A read answered 429, 500 or 503 is asked again up to 6 times, waiting from
 * 256 ms and doubling, and so is a broadcast answered 429, so a rate limit slows
 * a pass rather than failing it.
 *
 * @packageDocumentation
 * @module Esplora
 */

/** Transaction confirmation status as returned by esplora. */
export interface TxStatus {
  confirmed: boolean;
  block_height?: number;
}

/** Minimal transaction record returned by address- and scripthash-lookup endpoints. */
export interface EsploraTx {
  txid: string;
  status: TxStatus;
}

/**
 * Whether a specific output has been spent, and - if so - the spending transaction
 * and its status.
 */
export interface Outspend {
  spent: boolean;
  txid?: string;
  vin?: number;
  status?: TxStatus;
}

/**
 * One transaction of an address's history: its raw hex and confirming height,
 * `null` in the mempool. What the WASM driver's `ChainClient` reads.
 */
export interface HistoryTx {
  hex: string;
  height: number | null;
}

/**
 * The chain reads a swap needs, as {@link BitcoinEsplora} and
 * {@link LiquidEsplora} provide them.
 *
 * Program against this rather than a concrete class, and implement it yourself
 * to drive the SDK from your own indexer.
 */
export interface Esplora {
  /** Raw transaction hex, or `null` if not found. */
  getTx(txid: string): Promise<string | null>;
  /** Miner fee paid by a transaction, in satoshis, or `null` if not found. */
  getTxFee(txid: string): Promise<number | null>;
  /** Broadcasts a raw transaction (hex). Returns the broadcast txid. */
  broadcast(txHex: string): Promise<string>;
  /** Current chain tip height. */
  getHeight(): Promise<number>;
  /** Block hash at a given height, hex-encoded. */
  getBlockHash(height: number): Promise<string>;
  /** Confirmation status of a transaction. */
  getTxStatus(txid: string): Promise<TxStatus>;
  /** Whether a specific output has been spent, and by which transaction. */
  getOutspend(txid: string, vout: number): Promise<Outspend>;
  /** Number of confirmations for a transaction, or `0` if unconfirmed. */
  getTxConfirmations(txid: string): Promise<number>;
  /** Fee rate in sat/vB for a confirmation target, in blocks. Fractional. */
  getFeeRate(target: number): Promise<number>;
  /** Transactions paying a scripthash, newest first. */
  scripthashTxs(scriptHash: string, lastSeen?: string): Promise<EsploraTx[]>;
  /** Transactions paying an address, newest first. */
  addressTxs(address: string, lastSeen?: string): Promise<EsploraTx[]>;
  /**
   * Every transaction paying or spending an address, mempool included. Needed
   * only to serve the driver as its `ChainClient`.
   */
  addressHistory?(address: string): Promise<HistoryTx[]>;
  /**
   * The transaction spending an output, mempool included, or `undefined`
   * while it is unspent. Needed only to serve the driver as its
   * `ChainClient`.
   */
  outspend?(txid: string, vout: number): Promise<HistoryTx | undefined>;
}

/** How an Esplora client makes its requests. */
export interface EsploraOptions {
  /**
   * Seconds each try of a request may go unanswered before it fails, above
   * zero. Defaults to 30. A `fetch` passed in gets the timeout in `init.signal`,
   * so its own error handling sees the `TimeoutError`.
   */
  timeout?: number;
  /**
   * Aborts every request of this client, beside the timeout, for good once
   * aborted. Aborting is safe at any time, but a swap-out that is `Claiming`
   * needs passes that finish, or the server's CSV refund can take the opening.
   */
  signal?: AbortSignal;
}

const MAX_RETRIES = 6;
const BASE_BACKOFF_MS = 256;
const READ_RETRY_STATUSES: readonly number[] = [429, 500, 503];
/** A rate limit turns a broadcast away unread; a 500 or 503 may be its answer. */
const BROADCAST_RETRY_STATUSES: readonly number[] = [429];

/** Waits `ms`, or rejects with the reason as soon as `signal` aborts. */
function backoff(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const abort = () => {
      clearTimeout(timer);
      reject(signal?.reason);
    };
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", abort);
      resolve();
    }, ms);
    if (signal?.aborted) abort();
    else signal?.addEventListener("abort", abort, { once: true });
  });
}

/**
 * Everything the two chains share: one HTTP client over the Esplora routes. A
 * subclass supplies its chain's relay minimum.
 */
abstract class EsploraBase implements Esplora {
  private readonly baseUrl: string;
  private readonly fetchFn: typeof fetch;
  /**
   * Bodies the driver's reads already fetched, by txid. A body never changes
   * under its txid, so a read fetches only what it has not seen.
   */
  private readonly txs = new Map<string, string>();

  /** Relay minimum in sat/vB, the floor under every estimate. */
  protected readonly minFeeRate: number = 1;
  /** The rate when the indexer has no estimate within the target; none throws. */
  protected readonly fallbackFeeRate?: number;

  /**
   * @param url Esplora base URL including any path prefix.
   * @param fetchFn Optional `fetch` implementation. Defaults to the global `fetch`.
   * @param options Optional request settings, see {@link EsploraOptions}.
   */
  constructor(
    url: string,
    fetchFn: typeof fetch = fetch,
    options: EsploraOptions = {},
  ) {
    this.baseUrl = url.replace(/\/+$/, "");
    const seconds = options.timeout ?? 30;
    if (!(seconds > 0)) {
      throw new RangeError(`timeout must be above zero, got ${seconds}`);
    }
    this.fetchFn = async (input, init) => {
      const signals = [init?.signal, options.signal].filter(
        (signal): signal is AbortSignal => signal != null,
      );
      const aborted =
        signals.length > 1 ? AbortSignal.any(signals) : signals[0];
      const send = () => {
        const timeout = AbortSignal.timeout(seconds * 1000);
        const signal = aborted ? AbortSignal.any([aborted, timeout]) : timeout;
        return fetchFn(input, { ...init, signal });
      };
      const retryable =
        init?.method === "POST"
          ? BROADCAST_RETRY_STATUSES
          : READ_RETRY_STATUSES;
      let res = await send();
      for (let i = 0; i < MAX_RETRIES && retryable.includes(res.status); i++) {
        await res.body?.cancel();
        await backoff(BASE_BACKOFF_MS * 2 ** i, aborted);
        res = await send();
      }
      return res;
    };
  }

  /** Returns the raw transaction as hex, or `null` if not found. */
  async getTx(txid: string): Promise<string | null> {
    const res = await this.fetchFn(`${this.baseUrl}/tx/${txid}/hex`);
    if (res.status === 404) return null;
    if (!res.ok) throw new Error(`esplora get_tx: ${res.status}`);
    return (await res.text()).trim();
  }

  /** Miner fee paid by a transaction, in satoshis, or `null` if not found. */
  async getTxFee(txid: string): Promise<number | null> {
    const res = await this.fetchFn(`${this.baseUrl}/tx/${txid}`);
    if (res.status === 404) return null;
    if (!res.ok) throw new Error(`esplora get_tx_fee: ${res.status}`);
    const tx = (await res.json()) as { fee: number };
    return tx.fee;
  }

  /** Broadcasts a raw transaction (hex). Returns the broadcast txid. */
  async broadcast(txHex: string): Promise<string> {
    const res = await this.fetchFn(`${this.baseUrl}/tx`, {
      method: "POST",
      body: txHex,
    });
    const body = await res.text();
    if (!res.ok) throw new Error(`broadcast failed (${res.status}): ${body}`);
    return body.trim();
  }

  /** Current chain tip height. */
  async getHeight(): Promise<number> {
    const res = await this.fetchFn(`${this.baseUrl}/blocks/tip/height`);
    if (!res.ok) throw new Error(`esplora get_height: ${res.status}`);
    const text = await res.text();
    return parseInt(text.trim(), 10);
  }

  /**
   * Block hash at a given height, hex-encoded.
   *
   * Height `0` is the genesis hash a
   * {@link Context!SwapContext | SwapContext} needs for Liquid. Read it from your
   * own Esplora, never from the swap server: every taproot signature the wallet
   * produces commits to it.
   */
  async getBlockHash(height: number): Promise<string> {
    const res = await this.fetchFn(`${this.baseUrl}/block-height/${height}`);
    if (!res.ok) throw new Error(`esplora get_block_hash: ${res.status}`);
    return (await res.text()).trim();
  }

  /** Confirmation status of a transaction. */
  async getTxStatus(txid: string): Promise<TxStatus> {
    const res = await this.fetchFn(`${this.baseUrl}/tx/${txid}/status`);
    if (!res.ok) throw new Error(`esplora get_tx_status: ${res.status}`);
    return (await res.json()) as TxStatus;
  }

  /** Whether a specific output has been spent, and by which transaction. */
  async getOutspend(txid: string, vout: number): Promise<Outspend> {
    const res = await this.fetchFn(
      `${this.baseUrl}/tx/${txid}/outspend/${vout}`,
    );
    if (!res.ok) throw new Error(`esplora get_outspend: ${res.status}`);
    return (await res.json()) as Outspend;
  }

  /** Number of confirmations for a transaction, or `0` if unconfirmed. */
  async getTxConfirmations(txid: string): Promise<number> {
    const status = await this.getTxStatus(txid);
    if (!status.confirmed || status.block_height == null) return 0;
    const tip = await this.getHeight();
    return tip - status.block_height + 1;
  }

  /**
   * Estimates the fee rate in sat/vB for a given confirmation target (in
   * blocks), fractional, floored at the chain's relay minimum. The indexer
   * reports a fixed set of targets, so this takes the cheapest one that still
   * confirms within `target`. With none, it throws on Bitcoin and takes the
   * relay minimum on Liquid.
   */
  async getFeeRate(target: number): Promise<number> {
    const res = await this.fetchFn(`${this.baseUrl}/fee-estimates`);
    if (!res.ok) throw new Error(`esplora get_fee_rate: ${res.status}`);
    const estimates: Record<string, number> = (await res.json()) as Record<
      string,
      number
    >;
    const estimate = Object.entries(estimates)
      .map(([blocks, rate]) => ({ blocks: Number(blocks), rate }))
      .filter(
        ({ blocks, rate }) =>
          blocks <= target && Number.isFinite(rate) && rate >= 0,
      )
      .sort((a, b) => b.blocks - a.blocks)
      .at(0);
    const rate = estimate?.rate ?? this.fallbackFeeRate;
    if (rate === undefined) {
      throw new Error(
        `esplora get_fee_rate: no estimate within ${target} blocks`,
      );
    }
    return Math.max(rate, this.minFeeRate);
  }

  /** Transactions paying a scripthash, newest first. `lastSeen` pages past 25. */
  async scripthashTxs(
    scriptHash: string,
    lastSeen?: string,
  ): Promise<EsploraTx[]> {
    const path = lastSeen
      ? `/scripthash/${scriptHash}/txs/chain/${lastSeen}`
      : `/scripthash/${scriptHash}/txs`;
    const res = await this.fetchFn(`${this.baseUrl}${path}`);
    if (!res.ok) throw new Error(`esplora scripthash_txs: ${res.status}`);
    return (await res.json()) as EsploraTx[];
  }

  /** Transactions paying an address, newest first. `lastSeen` pages past 25. */
  async addressTxs(address: string, lastSeen?: string): Promise<EsploraTx[]> {
    const path = lastSeen
      ? `/address/${address}/txs/chain/${lastSeen}`
      : `/address/${address}/txs`;
    const res = await this.fetchFn(`${this.baseUrl}${path}`);
    if (!res.ok) throw new Error(`esplora address_txs: ${res.status}`);
    return (await res.json()) as EsploraTx[];
  }

  /**
   * Every transaction paying or spending `address`, mempool included, with its
   * raw hex and confirming height. Pages through the whole history, so a
   * deposit can never hide behind unrelated transactions; a transaction the
   * indexer has dropped since listing it is left out. Bodies are fetched one
   * at a time and kept as each arrives, so a later read fetches only what it
   * has not seen, and one that fails partway leaves the next less to fetch.
   */
  async addressHistory(address: string): Promise<HistoryTx[]> {
    const txs: EsploraTx[] = [];
    let page = await this.addressTxs(address);
    for (;;) {
      txs.push(...page);
      const confirmed = page.filter((tx) => tx.status.confirmed);
      const last = confirmed[confirmed.length - 1];
      if (confirmed.length < 25 || !last) break;
      page = await this.addressTxs(address, last.txid);
    }
    const history: HistoryTx[] = [];
    for (const tx of txs) {
      const hex = await this.knownTx(tx.txid);
      if (hex !== null) {
        history.push({ hex, height: tx.status.block_height ?? null });
      }
    }
    return history;
  }

  /**
   * The transaction spending output `vout` of `txid`, mempool included, with
   * its raw hex and confirming height, or `undefined` while the output is
   * unspent. Read from the output itself, so a spend is found however many
   * other transactions crowd the address's mempool history, which Esplora
   * caps at 50. A spend the indexer no longer serves counts as none.
   */
  async outspend(txid: string, vout: number): Promise<HistoryTx | undefined> {
    const spend = await this.getOutspend(txid, vout);
    if (!spend.spent || !spend.txid) return undefined;
    const hex = await this.knownTx(spend.txid);
    if (hex === null) return undefined;
    return { hex, height: spend.status?.block_height ?? null };
  }

  /** {@link getTx} through the driver's cache of bodies. */
  private async knownTx(txid: string): Promise<string | null> {
    const known = this.txs.get(txid);
    if (known !== undefined) return known;
    const hex = await this.getTx(txid);
    if (hex !== null) this.txs.set(txid, hex);
    return hex;
  }
}

/** Esplora client for Bitcoin. */
export class BitcoinEsplora extends EsploraBase {}

/** Esplora client for Liquid. */
export class LiquidEsplora extends EsploraBase {
  /** Elements' relay minimum, a tenth of Bitcoin's. */
  protected readonly minFeeRate: number = 0.1;
  protected readonly fallbackFeeRate: number = 0.1;
}
