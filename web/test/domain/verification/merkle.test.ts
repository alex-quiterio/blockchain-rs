import assert from "node:assert/strict";
import { test } from "node:test";
import { EMPTY_ROOT, climb, leaf, root } from "@domain/verification/merkle";
import { mined, proof } from "../../fixtures.ts";

test("a block's Merkle root is worked out as the node did", async () => {
  assert.equal(await root(mined.block.transactions), mined.block.merkle_root);
});

test("no transactions have the all-zero root, and one is its own root", async () => {
  assert.equal(await root([]), EMPTY_ROOT);
  const [payout] = mined.block.transactions;
  assert.equal(await root([payout!]), await leaf(payout!));
});

test("reordering or dropping a transaction changes the root", async () => {
  const txs = mined.block.transactions;
  assert.notEqual(await root([...txs].reverse()), mined.block.merkle_root);
  assert.notEqual(await root(txs.slice(0, -1)), mined.block.merkle_root);
});

test("the node's proof climbs from the transaction's leaf to the block's root", async () => {
  const leafHash = await leaf(proof.transaction);
  assert.equal(
    leafHash,
    proof.leaf,
    "the leaf is hashed as the node hashed it",
  );
  const path = await climb(leafHash, proof.proof);
  assert.equal(path.length, proof.proof.length + 1);
  assert.equal(path.at(-1), proof.header.merkle_root);
  assert.equal(proof.header.merkle_root, mined.block.merkle_root);
});

test("a proof fails for another transaction, or with a step on the wrong side", async () => {
  const other = mined.block.transactions[1]!;
  assert.notEqual(
    (await climb(await leaf(other), proof.proof)).at(-1),
    proof.header.merkle_root,
  );
  const flipped = proof.proof.map((s, i) =>
    i
      ? s
      : {
          ...s,
          side: s.side === "left" ? ("right" as const) : ("left" as const),
        },
  );
  assert.notEqual(
    (await climb(proof.leaf, flipped)).at(-1),
    proof.header.merkle_root,
  );
});
