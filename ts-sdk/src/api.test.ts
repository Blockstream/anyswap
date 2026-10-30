import { describe, expect, it } from "vitest";
import { AnySwapClient } from "./api";

describe("AnySwapClient", () => {
  it("refuses a response over the size limit", async () => {
    const body = JSON.stringify({ online: true }).padEnd(4 * 1024 * 1024 + 1);
    const client = new AnySwapClient("http://server", {
      fetchFn: async () => new Response(body),
    });

    await expect(client.getInfo()).rejects.toThrow("server response over");
  });

  it("reads a response under the size limit", async () => {
    const client = new AnySwapClient("http://server", {
      fetchFn: async () => new Response(JSON.stringify({ online: true })),
    });

    await expect(client.getInfo()).resolves.toEqual({ online: true });
  });
});
