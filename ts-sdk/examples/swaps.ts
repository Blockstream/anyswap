// Any number of swaps in both directions and on both chains, driven together
// by one driver: the shape of the Rust SDK's `examples/swaps.rs`.
//
// Usage: npx tsx examples/swaps.ts bcrt1q... 50000 lnbcrt1... liquid:el1qq... liquid:30000
//
// Arguments in any order. A number is a swap-out receiving that many sats, an
// `ln...` string is a swap-in invoice from your node, and anything else is the
// address the swaps of its chain pay out to: swap-out claims and swap-in
// refunds alike. A `liquid:` prefix puts an argument on Liquid instead of
// Bitcoin, with the genesis hash read from esplora and the asset id from the
// server. Pay the deposits and hold invoices by hand, watch the statuses,
// Ctrl-C when done. ANYSWAP_SERVER_URL, ANYSWAP_BITCOIN_ESPLORA_URL and, for
// Liquid swaps, ANYSWAP_LIQUID_ESPLORA_URL name the server and the Esplora
// APIs to run against.

import {
  initializeWasm,
  AnySwapClient,
  Wallet,
  SwapContext,
  Driver,
  MemoryStore,
  Chain,
  generateMnemonic,
  amountMsatFromInvoice,
  destAsset,
  type Destinations,
  type SwapEndpoint,
} from "../src";
import { BitcoinEsplora, LiquidEsplora } from "../src/esplora";

function env(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is not set`);
  return value;
}

interface Spec {
  chain: Chain;
  value: string;
}

const isAmount = (value: string) => /^\d+$/.test(value);
const isInvoice = (value: string) => value.startsWith("ln");

async function main() {
  await initializeWasm();

  const addresses = new Map<Chain, string>();
  const swaps: Spec[] = [];
  for (const arg of process.argv.slice(2)) {
    const [chain, value] = arg.startsWith("liquid:")
      ? [Chain.Liquid, arg.slice("liquid:".length)]
      : [Chain.Bitcoin, arg];
    if (isAmount(value) || isInvoice(value)) swaps.push({ chain, value });
    else addresses.set(chain, value);
  }
  if (swaps.length === 0) {
    console.error("usage: swaps [addresses] [liquid:]<sats or bolt11>...");
    process.exit(1);
  }
  for (const { chain } of swaps) {
    if (!addresses.has(chain)) {
      console.error(`give a ${chain} address for the ${chain} swaps`);
      process.exit(1);
    }
  }
  // The addresses given on the command line, one per chain, for both roles.
  const destinations: Destinations = {
    claimAddress: (chain) => addresses.get(chain) ?? "",
    refundAddress: (chain) => addresses.get(chain) ?? "",
  };

  const serverUrl = env("ANYSWAP_SERVER_URL");
  const info = await new AnySwapClient(serverUrl).getInfo();
  // In production, restore from a stored mnemonic instead of generating one.
  const wallet = Wallet.fromMnemonic(generateMnemonic(24), info.policy.network);
  const client = new AnySwapClient(serverUrl, { wallet });
  const onLiquid = swaps.some(({ chain }) => chain === Chain.Liquid);
  const liquidEsplora = onLiquid
    ? new LiquidEsplora(env("ANYSWAP_LIQUID_ESPLORA_URL"))
    : undefined;
  // Taking the asset from the server is a regtest convenience only: a wrong
  // asset id passes every later check, so outside regtest pin the known asset
  // id in configuration instead of asking the counterparty.
  const ctx = new SwapContext(
    wallet,
    liquidEsplora
      ? {
          genesisHash: await liquidEsplora.getBlockHash(0),
          policyAsset: destAsset(info, Chain.Liquid).dest_asset_id,
        }
      : undefined,
  );

  const driver = new Driver(ctx, client, new MemoryStore())
    .withChain(
      Chain.Bitcoin,
      new BitcoinEsplora(env("ANYSWAP_BITCOIN_ESPLORA_URL")),
    )
    .withDestinations(destinations)
    .withSignals(client.signals());
  if (liquidEsplora) driver.withChain(Chain.Liquid, liquidEsplora);

  for (const { chain, value } of swaps) {
    const onChain: SwapEndpoint = { chain };
    if (isAmount(value)) {
      const quote = await driver.quote("lightning", onChain, {
        receive: BigInt(value),
      });
      const swap = await driver.swapOut(quote);
      console.log(`${swap.swapId}: pay ${await swap.claimInvoice()}`);
    } else {
      const receiveAmount = amountMsatFromInvoice(value) / 1000n;
      const quote = await driver.quote(onChain, "lightning", {
        receive: receiveAmount,
      });
      const swap = await driver.swapIn(quote, value);
      const { send_amount } = await swap.record();
      console.log(
        `${swap.swapId}: deposit ${send_amount} sats to ${await swap.address()}`,
      );
    }
  }

  const updates = driver.subscribe();
  driver.run();
  for await (const swap of updates) {
    const error = swap.error;
    if (error) {
      console.log(`${swap.swapId} error: ${error.message}`);
    } else {
      const record = await swap.record();
      const server = record.server_status ?? "unreachable";
      console.log(
        `${swap.swapId} ${JSON.stringify(record.status)} (server: ${server})`,
      );
    }
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
