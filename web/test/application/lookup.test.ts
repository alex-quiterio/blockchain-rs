import assert from "node:assert/strict";
import { test } from "node:test";
import {
  block,
  refreshContext,
  context,
  search,
  transaction,
} from "@application/lookup";
import { wire } from "@application/ports";
import { mined, proof } from "../fixtures.ts";
import { fakeNode } from "./fake_node.ts";

const known = mined.block.hash;
const fake = fakeNode({
  status: () => ({ height: 2, tip: known, weight: "512", pending: 0 }),
  consensus: () => ({ kind: "proof_of_work", reward: 50 }) as never,
  block: (id) => {
    if (id === known || id === 2 || id === "2") return mined.block;
    if (id === 1) return { ...mined.block, index: 1 };
    throw new Error("no block with that height or hash");
  },
  works: () => ["256"],
  transaction: () => ({
    transaction: proof.transaction,
    status: "confirmed",
    block: {
      height: 2,
      hash: known,
      position: 3,
      timestamp: mined.block.timestamp,
      confirmations: 1,
    },
  }),
  proof: () => proof,
});
wire({ node: fake.node });

test("a search tells a height, a transaction, a block hash and an address apart", async () => {
  assert.deepEqual(await search(" 12 "), { kind: "block", id: "12" });
  assert.deepEqual(await search("ab".repeat(64)), {
    kind: "tx",
    id: "ab".repeat(64),
  });
  assert.deepEqual(await search(known), { kind: "block", id: known });
  assert.deepEqual(
    await search("cd".repeat(32)),
    { kind: "address", id: "cd".repeat(32) },
    "not a block, so an address",
  );
  await assert.rejects(search("nonsense"), /Try a block height/);
});

test("the rules are fetched only when the tip moves", async () => {
  assert.equal(await refreshContext(), true);
  assert.equal(context.height, 2);
  assert.equal(await refreshContext(), false);
  assert.equal(fake.called("consensus").length, 1);
});

test("a block comes with the one before it and, when mined, the work it needed", async () => {
  const found = await block("2");
  assert.equal(found.prev?.index, 1);
  assert.equal(found.work, 256);
});

test("a confirmed transaction comes with its block and the proof it's in it", async () => {
  const found = await transaction(proof.transaction.signature);
  assert.equal(found.block?.hash, known);
  assert.equal(found.proof?.leaf, proof.leaf);
});
