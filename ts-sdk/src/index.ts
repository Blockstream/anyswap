/**
 * AnySwap SDK for atomic swaps between Lightning and on-chain Bitcoin (or Liquid).
 *
 * The SDK communicates with an AnySwap HTTP server to coordinate swap-in (on-chain to
 * Lightning) and swap-out (Lightning to on-chain) operations. The server acts as the
 * responder; the user always initiates.
 *
 * The main entry points are:
 *
 * - {@link Driver} - runs every swap: creates it, funds or pays it, claims or
 *   refunds it, and keeps its record in a {@link SwapStore}.
 * - {@link AnySwapClient} - client for the server, the driver's {@link ServerClient}.
 * - {@link Wallet} - BIP39/BIP32 HD wallet for deriving swap keys from a mnemonic.
 * - {@link SwapContext} - the wallet plus chain parameters, built once and given
 *   to the driver. Liquid swaps need its Liquid parameters.
 * - {@link Esplora} - blockchain clients, the driver's {@link ChainClient}s;
 *   replaceable if you have your own.
 *
 * The claim and refund builders under the driver are public in the Rust
 * `anyswap-sdk` crate.
 *
 * @packageDocumentation
 */

export { AnySwapClient, AnySwapError } from "./api";
export type {
  AnySwapAuth,
  AnySwapClientOptions,
  GetSwapsParams,
  SwapSignals,
  SubscribeOptions,
  Subscription,
} from "./api";
export { initializeWasm, isWasmInitialized } from "./wasm";
export type { InitInput } from "./wasm";

export { Wallet } from "./wallet";
export { SwapContext } from "./context";
export type { LiquidParams } from "./context";
export {
  Driver,
  DriverError,
  MemoryStore,
  SwapHandle,
  Updates,
} from "./driver";
export type {
  Cancellation,
  CancelReason,
  ChainClient,
  ChainConfig,
  Destinations,
  DriverConfig,
  DriverErrorCode,
  DriverStatus,
  Endpoint,
  Fund,
  FundState,
  Height,
  Quote,
  RestoreReport,
  RunState,
  Sent,
  ServerCancel,
  ServerClient,
  Signals,
  SwapRecord,
  SwapStore,
  Timestamp,
} from "./driver";

export {
  generateMnemonic,
  validateMnemonic,
  validateInvoice,
  paymentHashFromInvoice,
  amountMsatFromInvoice,
  expiryAtFromInvoice,
  networkFromInvoice,
  newSwapId,
  protocolVersion,
  swapAmounts,
  destAsset,
} from "./utils";

export { SwapType, SwapNetwork, Chain, ErrorCode, NATIVE_ASSET } from "./types";
export type {
  SwapInStatus,
  SwapOutStatus,
  SwapStatus,
  SwapErrorCode,
  OutPoint,
  SwapEndpoint,
  CreateSwapRequest,
  SwapWebhook,
  SwapInWebhook,
  SwapOutWebhook,
  SwapState,
  InfoResponse,
  WebhookInfo,
  Policy,
  AssetPolicyResponse,
  ChainPolicyResponse,
  LightningPolicyResponse,
  TimeoutResponse,
  FeeEstimate,
  SwapListResponse,
  SwapListQuery,
  Pagination,
  CancelRequest,
  RevealClaimRequest,
  RevealPreimageRequest,
  SwapStateUpdate,
  ApiError,
} from "./types";
