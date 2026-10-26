declare module "#wasm-loader" {
  import type { InitInput } from "@blockstream/anyswap-wasm";
  export function resolveWasmSource(): Promise<InitInput>;
}
