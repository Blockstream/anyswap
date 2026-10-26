/**
 * Node implementation of {@link resolveWasmSource}. Reads the `.wasm` binary from
 * the local `@blockstream/anyswap-wasm` package.
 */

import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

import type { InitInput } from "@blockstream/anyswap-wasm";

export async function resolveWasmSource(): Promise<InitInput> {
  const require = createRequire(import.meta.url);
  const wasmPath =
    require.resolve("@blockstream/anyswap-wasm/anyswap_wasm_bg.wasm");
  return readFileSync(wasmPath);
}
