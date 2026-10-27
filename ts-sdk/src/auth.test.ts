import { etc, schnorr } from "@noble/secp256k1";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { AnySwapClient } from "./api";
import {
  IDENTITY_PUBKEY_HEADER,
  SIGNATURE_HEADER,
  TIMESTAMP_HEADER,
  signRequest,
  type AuthHeaders,
  type RawBody,
} from "./auth";
import {
  Chain,
  NATIVE_ASSET,
  SwapNetwork,
  type CreateSwapRequest,
} from "./types";
import { Wallet } from "./wallet";
import { initializeWasm } from "./wasm";

const MNEMONIC =
  "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const textEncoder = new TextEncoder();

let wallet: Wallet;

beforeAll(async () => {
  await initializeWasm();
  wallet = Wallet.fromMnemonic(MNEMONIC, SwapNetwork.Regtest);
});

describe("signRequest", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("returns AnySwap auth headers with a valid Schnorr signature", async () => {
    const rawBody = '{"swap_id":"swap-1"}';
    const headers = await signRequest(
      wallet,
      "post",
      "/swap",
      rawBody,
      "swap-1",
      42.9,
    );

    expect(headers[IDENTITY_PUBKEY_HEADER]).toBe(wallet.identityPubkey);
    expect(headers[TIMESTAMP_HEADER]).toBe("42");
    expect(headers[SIGNATURE_HEADER]).toMatch(/^[0-9a-f]{128}$/);
    await expectSignatureValid(
      headers,
      "post",
      "/swap",
      textEncoder.encode(rawBody),
      "swap-1",
    );
  });

  it("uses the default timestamp and empty body", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_700_000_000_987);

    const headers = await signRequest(wallet, "GET", "/swap");

    expect(headers[TIMESTAMP_HEADER]).toBe("1700000000");
    await expectSignatureValid(headers, "GET", "/swap", new Uint8Array(), "");
  });

  it.each([
    {
      name: "ArrayBuffer",
      rawBody: Uint8Array.from([0, 1, 2, 255]).buffer,
      expectedBody: Uint8Array.from([0, 1, 2, 255]),
    },
    {
      name: "ArrayBufferView",
      rawBody: new Uint8Array(
        Uint8Array.from([99, 10, 20, 30, 88]).buffer,
        1,
        3,
      ),
      expectedBody: Uint8Array.from([10, 20, 30]),
    },
  ] satisfies Array<{
    name: string;
    rawBody: RawBody;
    expectedBody: Uint8Array;
  }>)(
    "signs the exact bytes of an $name body",
    async ({ rawBody, expectedBody }) => {
      const headers = await signRequest(
        wallet,
        "PATCH",
        "/swap/swap-1",
        rawBody,
        null,
        500,
      );

      await expectSignatureValid(
        headers,
        "PATCH",
        "/swap/swap-1",
        expectedBody,
        "",
      );
    },
  );

  it("does not verify when the body differs from the signed body", async () => {
    const headers = await signRequest(
      wallet,
      "PUT",
      "/swap/swap-1",
      Uint8Array.from([1, 2, 3]),
      "swap-1",
      700,
    );

    expect(
      await signatureValid(
        headers,
        "PUT",
        "/swap/swap-1",
        Uint8Array.from([1, 2, 4]),
        "swap-1",
      ),
    ).toBe(false);
  });
});

describe("AnySwapClient auth", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("keeps /info public", async () => {
    const fetchFn = vi.fn(async () => jsonResponse({ protocol_version: "1" }));
    const client = new AnySwapClient("http://server.test/", { fetchFn });

    await client.getInfo();

    expect(fetchFn).toHaveBeenCalledWith("http://server.test/v1/info");
  });

  it("signs POST /swap over the exact serialized body", async () => {
    const calls: Array<{ url: string; init?: RequestInit }> = [];
    const fetchFn = vi.fn(
      async (input: Parameters<typeof fetch>[0], init?: RequestInit) => {
        calls.push({ url: String(input), init });
        return jsonResponse({});
      },
    );
    const client = new AnySwapClient("http://server.test/", {
      fetchFn,
      wallet,
    });
    const body: CreateSwapRequest = {
      protocol_version: "1",
      swap_id: "5e759d89-caea-438f-aca5-ae8c3c7a07bd",
      user_id: "user-id",
      source_chain: Chain.Bitcoin,
      source_asset_id: NATIVE_ASSET,
      receive_amount: "1000",
      send_amount: "1010",
      claim_invoice: "lnbc...",
      source_refund_pubkey: "02".padEnd(66, "0"),
    };

    await client.createSwap(body);

    expect(calls[0]?.url).toBe("http://server.test/v1/swap");
    expect(calls[0]?.init?.method).toBe("POST");
    expect(calls[0]?.init?.body).toBe(JSON.stringify(body));
    await expectSignatureValid(
      calls[0]?.init?.headers as AuthHeaders,
      "POST",
      "/v1/swap",
      textEncoder.encode(String(calls[0]?.init?.body)),
      body.swap_id,
    );
  });

  it("signs GET /swap history including its query params", async () => {
    const calls: Array<{ url: string; init?: RequestInit }> = [];
    const fetchFn = vi.fn(
      async (input: Parameters<typeof fetch>[0], init?: RequestInit) => {
        calls.push({ url: String(input), init });
        return jsonResponse({ swaps: [], pagination: {} });
      },
    );
    const client = new AnySwapClient("http://server.test", {
      fetchFn,
      wallet,
    });

    await client.getSwaps({ userId: "user-id", limit: 10, offset: 5 });

    expect(calls[0]?.url).toBe(
      "http://server.test/v1/swap?user_id=user-id&limit=10&offset=5",
    );
    const headers = calls[0]?.init?.headers as AuthHeaders;
    await expectSignatureValid(
      headers,
      "GET",
      "/v1/swap?user_id=user-id&limit=10&offset=5",
      new Uint8Array(),
      "",
    );
    expect(
      await signatureValid(
        headers,
        "GET",
        "/v1/swap?user_id=user-id&limit=100&offset=5",
        new Uint8Array(),
        "",
      ),
    ).toBe(false);
  });

  it("adds fresh auth query params to websocket subscriptions", async () => {
    const urls: string[] = [];
    class FakeWebSocket {
      onopen: (() => void) | null = null;
      onmessage: ((event: { data: unknown }) => void) | null = null;
      onclose: (() => void) | null = null;

      constructor(url: string) {
        urls.push(url);
      }

      close() {}
    }
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const client = new AnySwapClient("http://server.test", { wallet });

    const sub = client.subscribe(() => {});
    await waitFor(() => urls.length > 0);
    sub.stop();

    const url = new URL(urls[0] ?? "");
    expect(url.protocol).toBe("ws:");
    expect(url.pathname).toBe(`/v1/subscribe/${wallet.userId}`);
    const headers = Object.fromEntries(
      url.searchParams.entries(),
    ) as AuthHeaders;
    await expectSignatureValid(
      headers,
      "GET",
      `/v1/subscribe/${wallet.userId}`,
      new Uint8Array(),
      "",
    );
  });

  it("requires a wallet for protected swap endpoints", async () => {
    const client = new AnySwapClient("http://server.test", {
      fetchFn: vi.fn(),
    });

    await expect(client.getSwap("swap-1")).rejects.toThrow(
      "a wallet is required",
    );
  });
});

async function expectSignatureValid(
  headers: AuthHeaders,
  method: string,
  target: string,
  rawBody: Uint8Array,
  swapId: string,
): Promise<void> {
  expect(await signatureValid(headers, method, target, rawBody, swapId)).toBe(
    true,
  );
}

async function signatureValid(
  headers: AuthHeaders,
  method: string,
  target: string,
  rawBody: Uint8Array,
  swapId: string,
): Promise<boolean> {
  const pubkey = etc.hexToBytes(headers[IDENTITY_PUBKEY_HEADER]);
  const message = etc.concatBytes(
    field(textEncoder.encode(method.toUpperCase())),
    field(textEncoder.encode(target)),
    await sha256(rawBody),
    field(textEncoder.encode(swapId)),
    int64BE(Number(headers[TIMESTAMP_HEADER])),
    pubkey,
  );
  return schnorr.verifyAsync(
    etc.hexToBytes(headers[SIGNATURE_HEADER]),
    await taggedHash("anyswap/auth/v1", message),
    pubkey.slice(1),
  );
}

function field(value: Uint8Array): Uint8Array {
  const out = new Uint8Array(4 + value.length);
  new DataView(out.buffer).setUint32(0, value.length, false);
  out.set(value, 4);
  return out;
}

function int64BE(value: number): Uint8Array {
  const out = new Uint8Array(8);
  new DataView(out.buffer).setBigInt64(0, BigInt(value), false);
  return out;
}

async function taggedHash(
  tag: string,
  message: Uint8Array,
): Promise<Uint8Array> {
  const tagHash = await sha256(textEncoder.encode(tag));
  return sha256(etc.concatBytes(tagHash, tagHash, message));
}

async function sha256(bytes: Uint8Array): Promise<Uint8Array> {
  return new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
}

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  });
}

async function waitFor(predicate: () => boolean): Promise<void> {
  for (let i = 0; i < 20; i++) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  throw new Error("timed out waiting for condition");
}
