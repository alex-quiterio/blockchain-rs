// Records what the verification tests check against: real blocks, a Merkle proof and a forged
// block, as a node makes them, so the page's checks are tested against the node and not only
// against themselves. Run it against fresh nodes, one of each consensus, then commit the files:
//   CONSENSUS=pow DIFFICULTY=2 BIND_ADDR=127.0.0.1:3901 CHAIN_FILE=/tmp/pow.json target/release/blockchain &
//   CONSENSUS=pos BIND_ADDR=127.0.0.1:3902 CHAIN_FILE=/tmp/pos.json target/release/blockchain &
//   node web/scripts/record_fixtures.ts http://127.0.0.1:3901 http://127.0.0.1:3902

import { writeFileSync } from "node:fs";

const [pow, pos] = process.argv.slice(2);
if (!pow || !pos)
  throw new Error("usage: record_fixtures.ts <pow node url> <pos node url>");

async function call<T>(
  base: string,
  method: "GET" | "POST",
  path: string,
  body?: unknown,
): Promise<T> {
  const res = await fetch(base + path, {
    method,
    headers: body ? { "content-type": "application/json" } : {},
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!res.ok)
    throw new Error(`${method} ${path}: ${res.status} ${await res.text()}`);
  return (await res.json()) as T;
}

type Wallet = { address: string; secret_key: string };
type Tx = { signature: string };

// Proof of work: a funded wallet pays three others, one with a fee, so the block holds four
// transactions and a proof of the last one has an odd hash out on the way up.
const payer = await call<Wallet>(pow, "POST", "/wallets");
await call(pow, "POST", "/mine", { miner: payer.address });
const signatures: string[] = [];
for (const [i, fee] of [0, 3, 0].entries()) {
  const to = await call<Wallet>(pow, "POST", "/wallets");
  const tx = await call<Tx>(pow, "POST", "/transactions/sign", {
    secret_key: payer.secret_key,
    to: to.address,
    amount: 5 + i,
    fee,
  });
  await call(pow, "POST", "/transactions", tx);
  signatures.push(tx.signature);
}
const mined = await call<{ index: number }>(pow, "POST", "/mine", {
  miner: payer.address,
});
const block = await call(pow, "GET", `/blocks/${mined.index}`);
const work = await call<string[]>(
  pow,
  "GET",
  `/chain/work?from=${mined.index}&limit=1`,
);
const proof = await call(pow, "GET", `/transactions/${signatures[2]}/proof`);

// Proof of stake: forge while anyone may, stake (which commits to a reveal chain), then forge as
// the drawn validator, whose block carries a reveal and a signature over its hash.
const validator = await call<Wallet>(pos, "POST", "/wallets");
await call(pos, "POST", "/forge", { secret_key: validator.secret_key });
const stake = await call<Tx>(pos, "POST", "/transactions/sign", {
  secret_key: validator.secret_key,
  to: "stake",
  amount: 10,
});
await call(pos, "POST", "/transactions", stake);
await call(pos, "POST", "/forge", { secret_key: validator.secret_key });
const forged = await call<{ index: number }>(pos, "POST", "/forge", {
  secret_key: validator.secret_key,
});
const forgedBlock = await call(pos, "GET", `/blocks/${forged.index}`);

const dir = new URL("../test/fixtures/", import.meta.url);
const save = (name: string, value: unknown) =>
  writeFileSync(new URL(name, dir), JSON.stringify(value, null, 2) + "\n");
save("mined_block.json", { block, work: Number(work[0]) });
save("proof.json", proof);
save("forged_block.json", forgedBlock);
console.log(
  `Recorded block ${mined.index}, a proof of its last transfer, and forged block ${forged.index}.`,
);
