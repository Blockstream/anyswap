import type {
  ChainClient,
  ServerClient,
  Signals,
} from "@blockstream/anyswap-wasm";
import { beforeAll, describe, expect, it, vi } from "vitest";
import { AnySwapClient, type SwapSignals } from "./api";
import { SwapContext } from "./context";
import { Driver, DriverError, MemoryStore } from "./driver";
import { BitcoinEsplora, type EsploraTx } from "./esplora";
import { Chain, SwapNetwork } from "./types";
import { generateMnemonic } from "./utils";
import { Wallet } from "./wallet";
import { initializeWasm } from "./wasm";

let wallet: Wallet;

beforeAll(async () => {
  await initializeWasm();
  wallet = Wallet.fromMnemonic(generateMnemonic(12), SwapNetwork.Regtest);
});

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

async function waitFor(predicate: () => boolean): Promise<void> {
  for (let i = 0; i < 100 && !predicate(); i++) {
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  expect(predicate()).toBe(true);
}

describe("the wasm driver's interfaces", () => {
  it("are satisfied by the client, its signals and the esplora as they are", () => {
    const client = new AnySwapClient("http://server.test", { wallet });
    const server: ServerClient = client;
    const chain: ChainClient = new BitcoinEsplora("http://esplora.test");
    const signals: (s: SwapSignals) => Signals = (s) => s;
    expect(server).toBe(client);
    expect(chain).toBeInstanceOf(BitcoinEsplora);
    expect(signals).toBeTypeOf("function");
  });

  it("listSwaps sends the raw query and skips what is unset", async () => {
    const fetchFn = vi.fn(async (_input: Parameters<typeof fetch>[0]) =>
      json({ swaps: [], pagination: {} }),
    );
    const client = new AnySwapClient("http://server.test", {
      wallet,
      fetchFn,
    });

    await client.listSwaps({
      user_id: "u",
      limit: 100,
      offset: 0,
      is_active: null,
      status: "Claimed,Refunded",
      with_create_request: true,
    });

    const url = new URL(String(fetchFn.mock.calls[0]?.[0]));
    expect(url.pathname).toBe("/v1/swap");
    expect([...url.searchParams.entries()]).toEqual([
      ["user_id", "u"],
      ["limit", "100"],
      ["offset", "0"],
      ["status", "Claimed,Refunded"],
      ["with_create_request", "true"],
    ]);
  });

  it("signals queues every pushed swap id until next takes it", async () => {
    let socket: FakeWebSocket | undefined;
    class FakeWebSocket {
      onopen: (() => void) | null = null;
      onmessage: ((event: { data: unknown }) => void) | null = null;
      onclose: (() => void) | null = null;

      constructor(_url: string) {
        socket = this;
      }

      close() {}
    }
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const client = new AnySwapClient("http://server.test", { wallet });
    const push = (swap_id: string) =>
      socket?.onmessage?.({ data: JSON.stringify({ swap_id, status: "x" }) });

    const signals = client.signals();
    await waitFor(() => socket !== undefined);
    socket?.onopen?.();
    push("a");
    push("b");
    expect(await signals.next()).toBe("a");
    expect(await signals.next()).toBe("b");

    const pending = signals.next();
    push("c");
    expect(await pending).toBe("c");

    signals.stop();
    expect(await signals.next()).toBeUndefined();
    vi.unstubAllGlobals();
  });

  it("addressHistory pages through the chain and drops what the indexer no longer serves", async () => {
    const confirmed = (n: number): EsploraTx => ({
      txid: `c${n}`,
      status: { confirmed: true, block_height: 100 + n },
    });
    const page1: EsploraTx[] = [
      { txid: "m1", status: { confirmed: false } },
      { txid: "m2", status: { confirmed: false } },
      ...Array.from({ length: 25 }, (_, i) => confirmed(i + 1)),
    ];
    const page2 = [confirmed(26)];
    const fetchFn = vi.fn(async (input: Parameters<typeof fetch>[0]) => {
      const url = String(input);
      if (url.endsWith("/address/addr/txs")) return json(page1);
      if (url.endsWith("/address/addr/txs/chain/c25")) return json(page2);
      const txid = /\/tx\/(\w+)\/hex$/.exec(url)?.[1];
      if (txid === "m1") return new Response("", { status: 404 });
      if (txid) return new Response(`hex-${txid}`);
      throw new Error(`unexpected ${url}`);
    });

    const history = await new BitcoinEsplora(
      "http://esplora.test",
      fetchFn,
    ).addressHistory("addr");

    expect(history).toHaveLength(27);
    expect(history[0]).toEqual({ hex: "hex-m2", height: null });
    expect(history[1]).toEqual({ hex: "hex-c1", height: 101 });
    expect(history[26]).toEqual({ hex: "hex-c26", height: 126 });
  });

  it("outspend fetches the spending transaction and reads an unspent or unserved one as none", async () => {
    const fetchFn = vi.fn(async (input: Parameters<typeof fetch>[0]) => {
      const url = String(input);
      if (url.endsWith("/tx/a/outspend/0"))
        return json({
          spent: true,
          txid: "s",
          status: { confirmed: true, block_height: 7 },
        });
      if (url.endsWith("/tx/a/outspend/1"))
        return json({ spent: true, txid: "m", status: { confirmed: false } });
      if (url.endsWith("/tx/a/outspend/2")) return json({ spent: false });
      if (url.endsWith("/tx/a/outspend/3"))
        return json({ spent: true, txid: "gone" });
      if (url.endsWith("/tx/s/hex")) return new Response("hex-s");
      if (url.endsWith("/tx/m/hex")) return new Response("hex-m");
      if (url.endsWith("/tx/gone/hex"))
        return new Response("", { status: 404 });
      throw new Error(`unexpected ${url}`);
    });
    const esplora = new BitcoinEsplora("http://esplora.test", fetchFn);

    expect(await esplora.outspend("a", 0)).toEqual({ hex: "hex-s", height: 7 });
    expect(await esplora.outspend("a", 1)).toEqual({
      hex: "hex-m",
      height: null,
    });
    expect(await esplora.outspend("a", 2)).toBeUndefined();
    expect(await esplora.outspend("a", 3)).toBeUndefined();
  });
});

describe("Driver", () => {
  const chain: ChainClient = {
    getHeight: async () => 0,
    getFeeRate: async () => 1,
    addressHistory: async () => [],
    outspend: async () => undefined,
    broadcast: async () => "",
  };
  const driver = (server: Partial<ServerClient>) =>
    new Driver(
      new SwapContext(
        Wallet.fromMnemonic(generateMnemonic(12), SwapNetwork.Regtest),
      ),
      server as ServerClient,
      new MemoryStore(),
    ).withChain(Chain.Bitcoin, chain);
  const answering = (status: number) =>
    driver({
      getInfo: async () => {
        throw Object.assign(new Error("no"), { status });
      },
    });
  const quote = (d: Driver) =>
    d.quote("lightning", { chain: Chain.Bitcoin }, { receive: 1n });

  it("get resolves undefined for a swap the store lacks", async () => {
    const swap = await driver({}).get("11111111-1111-4111-8111-111111111111");
    expect(swap).toBeUndefined();
  });

  it("throws DriverError instances that are Errors", async () => {
    const error = await driver({})
      .get("nope")
      .catch((e: unknown) => e);
    expect(error).toBeInstanceOf(DriverError);
    expect(error).toBeInstanceOf(Error);
    expect(error).toMatchObject({
      code: "Invalid",
      retryable: false,
      httpStatus: undefined,
    });
  });

  it("reads a thrown status as the server's answer and any other throw as transport", async () => {
    await expect(quote(answering(503))).rejects.toMatchObject({
      code: "Internal",
      retryable: true,
      httpStatus: 503,
    });
    await expect(quote(answering(400))).rejects.toMatchObject({
      code: "BadRequest",
      retryable: false,
      httpStatus: 400,
    });
    const down = driver({
      getInfo: async () => {
        throw new Error("fetch failed");
      },
    });
    await expect(quote(down)).rejects.toMatchObject({
      code: "Transport",
      retryable: true,
      httpStatus: undefined,
    });
  });

  it("keeps the server's error code beside its status", async () => {
    const expired = driver({
      getInfo: async () => {
        throw Object.assign(new Error("stale"), {
          status: 410,
          code: "QuoteExpired",
        });
      },
    });
    await expect(quote(expired)).rejects.toMatchObject({
      code: "QuoteExpired",
      httpStatus: 410,
    });
  });

  it("takes a config and per-chain overrides", () => {
    const configured = driver({})
      .withConfig({ pollInterval: 5, feeRateCacheTtl: 0, refundTimeout: 60 })
      .withChain(Chain.Bitcoin, chain, { minConfs: 1, blockTime: 1 });
    expect(configured.state).toBe("Idle");
  });

  it("stops for good, before or after run", async () => {
    const idle = driver({});
    idle.stop();
    expect(idle.state).toBe("Stopped");
    await idle.waitStopped();

    const running = driver({});
    running.run();
    running.stop();
    await running.waitStopped();
    expect(running.state).toBe("Stopped");
    running.run();
    expect(running.state).toBe("Stopped");
  });
});
