import assert from "node:assert/strict";
import { test } from "node:test";
import {
  auditAddress,
  auditBlock,
  auditTransaction,
  signatureOf,
  totalsOf,
} from "@application/audit";
import { Lookup } from "@application/lookup";
import type { Consensus, Located } from "@application/ports";
import type { Block } from "@domain/blocks";
import { COINBASE, STAKE } from "@domain/transfers";
import { forged, mined, proof } from "../fixtures.ts";
import { account, fakeNode } from "./fake_node.ts";

/** Lookups through a node that answers from `blocks`, by height or hash, as it would. */
async function serving(blocks: Block[], placedHash = mined.block.hash) {
  const find = (id: string | number) => {
    const found = blocks.find(
      (b) => String(b.index) === String(id) || b.hash === id,
    );
    if (!found) throw new Error("no block with that height or hash");
    return found;
  };
  const fake = fakeNode({
    status: () => ({
      chain: "00".repeat(32),
      height: 3,
      tip: "ff".repeat(32),
      weight: "1",
      pending: 0,
    }),
    consensus: () => ({ kind: "proof_of_work", reward: 50 }) as Consensus,
    block: find,
    works: () => [String(mined.work)],
    transaction: () => ({
      transaction: proof.transaction,
      status: "confirmed",
      block: {
        height: 2,
        hash: placedHash,
        position: 3,
        timestamp: mined.block.timestamp,
        confirmations: 1,
      },
    }),
    proof: () => proof,
  });
  const lookup = new Lookup(fake.node);
  await lookup.refreshContext();
  return lookup;
}

// A block before the mined one, whose hash the mined one names.
const before: Block = {
  ...mined.block,
  index: 1,
  hash: mined.block.prev_hash,
  transactions: [],
};

test("a block the node mined passes every check", async () => {
  const lookup = await serving([before, mined.block]);
  const audit = await auditBlock(lookup, "2");
  assert.equal(await audit.header, true);
  assert.deepEqual(
    audit.link && { passed: audit.link.passed, prev: audit.link.prev.index },
    { passed: true, prev: 1 },
  );
  assert.equal(audit.seal.by, "work");
  assert.equal(audit.seal.passed, true);
  assert.equal(await audit.contents, true);
  assert.deepEqual(await audit.signatures, { verified: 3, of: 3 });
  assert.deepEqual(
    audit.payout && { passed: audit.payout.passed, fees: audit.payout.fees },
    { passed: true, fees: 3 },
  );
});

test("a forged block's seal is its validator's signature", async () => {
  const lookup = await serving([
    {
      ...forged,
      index: forged.index - 1,
      hash: forged.prev_hash,
      transactions: [],
    },
    forged,
  ]);
  const audit = await auditBlock(lookup, String(forged.index));
  assert.equal(audit.seal.by, "signature");
  assert.equal(await audit.seal.passed, true);
  assert.equal(await audit.header, true);
});

test("a tampered block fails exactly the checks it breaks", async () => {
  const [payout, ...rest] = mined.block.transactions;
  const padded = {
    ...mined.block,
    transactions: [{ ...payout!, amount: payout!.amount + 1 }, ...rest],
  };
  const lookup = await serving([{ ...before, hash: "00".repeat(32) }, padded]);
  const audit = await auditBlock(lookup, "2");
  assert.equal(
    audit.link?.passed,
    false,
    "it doesn't name the block before it",
  );
  assert.equal(
    audit.payout?.passed,
    false,
    "the payout isn't the reward plus fees",
  );
  assert.equal(
    await audit.contents,
    false,
    "its transactions no longer hash to its root",
  );
  assert.equal(await audit.header, true, "the header itself wasn't touched");
});

test("a confirmed transfer's signature and its inclusion check out", async () => {
  const lookup = await serving([mined.block]);
  const audit = await auditTransaction(lookup, proof.transaction.signature);
  assert.equal(await audit.signature, true);
  const inclusion = await audit.inclusion!;
  assert.ok(inclusion.leafMatches && inclusion.reachesRoot);
  assert.equal(await inclusion.header, true);
  assert.equal(inclusion.path.length, proof.proof.length + 1);
});

test("a proof whose header isn't the block the transfer is placed in fails", async () => {
  const lookup = await serving([mined.block], "ee".repeat(32));
  const inclusion = await (
    await auditTransaction(lookup, proof.transaction.signature)
  ).inclusion!;
  assert.equal(await inclusion.header, false);
});

test("a payout has no signature to check", async () => {
  assert.equal(await signatureOf(mined.block.transactions[0]!), null);
  assert.equal(await signatureOf(mined.block.transactions[1]!), true);
});

test("an address's history adds up to its balance, pending transfers left out", async () => {
  const me = "aa".repeat(32);
  const other = "bb".repeat(32);
  const confirmed = (tx: Located["transaction"]): Located => ({
    transaction: tx,
    status: "confirmed",
    block: {
      height: 1,
      hash: "",
      position: 0,
      timestamp: "",
      confirmations: 1,
    },
  });
  const tx = (from: string, to: string, amount: number, fee?: number) => ({
    from,
    to,
    amount,
    nonce: 0,
    signature: "",
    ...(fee ? { fee } : {}),
  });
  const history: Located[] = [
    confirmed(tx(COINBASE, me, 50)),
    confirmed(tx(other, me, 7)),
    confirmed(tx(me, other, 10, 2)),
    confirmed(tx(me, STAKE, 5)),
    { transaction: tx(me, other, 99), status: "pending", block: null },
  ];
  assert.deepEqual(totalsOf(me, history), {
    rewards: 50,
    received: 7,
    sent: 10,
    fees: 2,
    staked: 5,
  });

  const fake = fakeNode({
    balance: () => account({ address: me, balance: 40, staked: 5 }),
    history: () => history,
  });
  assert.equal((await auditAddress(new Lookup(fake.node), me)).addsUp, true);
  const off = fakeNode({
    balance: () => account({ address: me, balance: 41, staked: 5 }),
    history: () => history,
  });
  assert.equal((await auditAddress(new Lookup(off.node), me)).addsUp, false);
});
