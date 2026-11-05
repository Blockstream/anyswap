import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BitcoinEsplora } from "./esplora";

const BASE_URL = "http://esplora.test";

function answering(...statuses: number[]) {
  return vi.fn(
    async (_input: Parameters<typeof fetch>[0], _init?: RequestInit) =>
      new Response("7", { status: statuses.shift() ?? 200 }),
  );
}

describe("esplora retries", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("asks a read again on 429, 500 and 503, doubling the wait", async () => {
    const fetchFn = answering(429, 500, 503, 200);

    const height = new BitcoinEsplora(BASE_URL, fetchFn).getHeight();

    await vi.advanceTimersByTimeAsync(255);
    expect(fetchFn).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(fetchFn).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(511);
    expect(fetchFn).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(1);
    expect(fetchFn).toHaveBeenCalledTimes(3);
    await vi.advanceTimersByTimeAsync(1024);
    expect(fetchFn).toHaveBeenCalledTimes(4);
    expect(await height).toBe(7);
  });

  it("gives up on a read after 6 retries with the last answer", async () => {
    const fetchFn = answering(...Array<number>(8).fill(429));

    const height = new BitcoinEsplora(BASE_URL, fetchFn).getHeight();
    const failed = expect(height).rejects.toThrow("esplora get_height: 429");
    await vi.runAllTimersAsync();

    await failed;
    expect(fetchFn).toHaveBeenCalledTimes(7);
  });

  it("takes any other answer at once", async () => {
    const fetchFn = answering(404);

    expect(await new BitcoinEsplora(BASE_URL, fetchFn).getTx("a")).toBeNull();
    expect(fetchFn).toHaveBeenCalledTimes(1);
  });

  it("sends a broadcast again on 429", async () => {
    const fetchFn = answering(429, 200);

    const txid = new BitcoinEsplora(BASE_URL, fetchFn).broadcast("hex");
    await vi.runAllTimersAsync();

    expect(await txid).toBe("7");
    expect(fetchFn).toHaveBeenCalledTimes(2);
    expect(fetchFn.mock.calls[1][1]?.body).toBe("hex");
  });

  it.each([500, 503])(
    "does not send a broadcast again on %i",
    async (status) => {
      const fetchFn = answering(status);

      await expect(
        new BitcoinEsplora(BASE_URL, fetchFn).broadcast("hex"),
      ).rejects.toThrow(`broadcast failed (${status})`);
      expect(fetchFn).toHaveBeenCalledTimes(1);
    },
  );

  it("gives each try its own timeout", async () => {
    const fetchFn = answering(429, 200);

    const height = new BitcoinEsplora(BASE_URL, fetchFn).getHeight();
    await vi.runAllTimersAsync();
    await height;

    const [first, second] = fetchFn.mock.calls.map(([, init]) => init?.signal);
    expect(first).toBeInstanceOf(AbortSignal);
    expect(second).toBeInstanceOf(AbortSignal);
    expect(first).not.toBe(second);
  });

  it("ends the wait when the client is aborted", async () => {
    const abort = new AbortController();
    const fetchFn = answering(429, 200);

    const height = new BitcoinEsplora(BASE_URL, fetchFn, {
      signal: abort.signal,
    }).getHeight();
    const failed = expect(height).rejects.toThrow("stopped");
    await vi.advanceTimersByTimeAsync(0);
    abort.abort(new Error("stopped"));

    await failed;
    expect(fetchFn).toHaveBeenCalledTimes(1);
  });
});
