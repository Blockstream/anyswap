// One swap, driven by hand to its outcome, over records that survive a
// restart: the shape of the Rust SDK's `examples/advance.rs`, with the store
// and the loop an integrator writes.
//
// Usage:
//   npx tsx examples/advance.ts in <bolt11> <refund address>
//   npx tsx examples/advance.ts out <sats> <claim address>
//   npx tsx examples/advance.ts resume <swap id>
//   npx tsx examples/advance.ts list
//
// `in` creates a swap-in paying the invoice and prints the deposit to make,
// `out` a swap-out of that many sats to the address and prints the hold
// invoice to pay; both then loop over `advance` until the swap settles. The
// mnemonic and one JSON file per record live under `./anyswap-advance`, so
// `resume` continues a swap after a restart and `list` shows every one on
// file. Bitcoin regtest; see swaps.ts for the Liquid setup.
// ANYSWAP_SERVER_URL and ANYSWAP_BITCOIN_ESPLORA_URL name the server and the
// Esplora API to run against.

import { mkdirSync } from "node:fs";
import { readdir, readFile, rename, writeFile } from "node:fs/promises";
import { join } from "node:path";
import {
  initializeWasm,
  AnySwapClient,
  Wallet,
  SwapContext,
  Driver,
  DriverError,
  Chain,
  SwapNetwork,
  generateMnemonic,
  amountMsatFromInvoice,
  type DriverStatus,
  type Fund,
  type SwapHandle,
  type SwapRecord,
  type SwapStore,
} from "../src";
import { BitcoinEsplora } from "../src/esplora";

const NETWORK = SwapNetwork.Regtest;
const DIR = "anyswap-advance";

function env(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is not set`);
  return value;
}

// One JSON file per record, the record as given, written whole and renamed
// into place so a crash mid-write leaves the old record.
class FileStore implements SwapStore {
  constructor(private readonly dir: string) {}

  private path(swapId: string): string {
    return join(this.dir, `${swapId}.json`);
  }

  async save(record: SwapRecord): Promise<void> {
    const path = this.path(record.swap_id);
    await writeFile(`${path}.tmp`, JSON.stringify(record, null, 2));
    await rename(`${path}.tmp`, path);
  }

  async load(swapId: string): Promise<SwapRecord | undefined> {
    try {
      return JSON.parse(
        await readFile(this.path(swapId), "utf8"),
      ) as SwapRecord;
    } catch (e) {
      if ((e as NodeJS.ErrnoException).code === "ENOENT") return undefined;
      throw e;
    }
  }

  async list(): Promise<string[]> {
    return (await readdir(this.dir))
      .filter((n) => n.endsWith(".json"))
      .map((n) => n.slice(0, -".json".length));
  }
}

// The mnemonic on file, made on the first run. Regtest only: a real
// integration keeps it where it keeps the wallet's.
async function mnemonic(dir: string): Promise<string> {
  const path = join(dir, "mnemonic");
  try {
    return (await readFile(path, "utf8")).trim();
  } catch {
    const words = generateMnemonic(12);
    await writeFile(path, words);
    return words;
  }
}

async function setup(
  dir: string,
): Promise<{ driver: Driver; store: FileStore }> {
  mkdirSync(dir, { recursive: true });
  const wallet = Wallet.fromMnemonic(await mnemonic(dir), NETWORK);
  const client = new AnySwapClient(env("ANYSWAP_SERVER_URL"), {
    auth: { identityPrivkey: wallet.identityPrivkey },
  });
  const store = new FileStore(dir);
  const driver = new Driver(new SwapContext(wallet), client, store)
    .withChain(
      Chain.Bitcoin,
      new BitcoinEsplora(env("ANYSWAP_BITCOIN_ESPLORA_URL")),
    )
    .withSignals(client.signals(wallet.userId));
  return { driver, store };
}

// A terminal status and, on a swap-in, every refund of ours confirmed with
// nothing left to take.
function settled(status: DriverStatus, funds: Fund[]): boolean {
  const terminal =
    status.status === "Completed" || status.status === "Canceled";
  return (
    terminal &&
    funds.every(({ state }) => ["Refunded", "Dust"].includes(state.state))
  );
}

// `advance` until the swap settles. A chain or server outage is waited out,
// since `advance` paces itself.
async function complete(swap: SwapHandle): Promise<DriverStatus> {
  for (;;) {
    try {
      const status = await swap.advance();
      console.log(JSON.stringify(status));
      if (settled(status, (await swap.record()).funds)) return status;
    } catch (e) {
      if (e instanceof DriverError && e.retryable) {
        console.log(`retrying: ${e.message}`);
      } else {
        throw e;
      }
    }
  }
}

async function main() {
  await initializeWasm();
  const [command, ...args] = process.argv.slice(2);
  const { driver, store } = await setup(DIR);
  switch (command) {
    case "in": {
      const [bolt11, refundAddress] = args;
      const receiveAmount = amountMsatFromInvoice(bolt11) / 1000n;
      const quote = await driver.quote({ chain: Chain.Bitcoin }, "lightning", {
        receive: receiveAmount,
      });
      const swap = await driver.swapIn(quote, bolt11, refundAddress);
      const { send_amount } = await swap.record();
      console.log(
        `${swap.swapId}: deposit ${send_amount} sats to ${await swap.address()}`,
      );
      await complete(swap);
      break;
    }
    case "out": {
      const [sats, claimAddress] = args;
      const quote = await driver.quote(
        "lightning",
        { chain: Chain.Bitcoin },
        { receive: BigInt(sats) },
      );
      const swap = await driver.swapOut(quote, claimAddress);
      console.log(`${swap.swapId}: pay ${await swap.claimInvoice()}`);
      await complete(swap);
      break;
    }
    case "resume": {
      const [swapId] = args;
      const swap = await driver.get(swapId);
      if (!swap) throw new Error(`no swap ${swapId} on file`);
      await complete(swap);
      break;
    }
    case "list": {
      for (const swapId of await store.list()) {
        const record = await store.load(swapId);
        if (!record) continue;
        const type = record.source.chain ? "SwapIn" : "SwapOut";
        console.log(`${swapId} ${type} ${JSON.stringify(record.status)}`);
      }
      break;
    }
    default:
      throw new Error(
        "usage: advance in <bolt11> <refund address> | out <sats> <claim address> | resume <swap id> | list",
      );
  }
  process.exit(0);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
