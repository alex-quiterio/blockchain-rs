import assert from "node:assert/strict";
import { test } from "node:test";
import { headerHash, meetsWork } from "@domain/verification/blocks";
import { forged, mined } from "../../fixtures.ts";

test("a header hashes as the node hashed it, mined or forged", async () => {
  assert.equal(await headerHash(mined.block), mined.block.hash);
  assert.equal(
    await headerHash(forged),
    forged.hash,
    "the validator and reveal are hashed too",
  );
});

test("changing any header field changes the hash", async () => {
  for (const change of [
    { nonce: mined.block.nonce + 1 },
    { index: 9 },
    { merkle_root: "0".repeat(64) },
  ]) {
    assert.notEqual(
      await headerHash({ ...mined.block, ...change }),
      mined.block.hash,
    );
  }
});

test("a timestamp is hashed with all nine fractional digits, however its JSON trims them", async () => {
  const header = { ...mined.block, timestamp: "2026-10-30T10:00:00.5Z" };
  const padded = {
    ...mined.block,
    timestamp: "2026-10-30T10:00:00.500000000Z",
  };
  assert.equal(await headerHash(header), await headerHash(padded));
});

test("a mined block meets the work it needed, and not much more", () => {
  assert.ok(meetsWork(mined.block.hash, mined.work));
  assert.equal(meetsWork(mined.block.hash, mined.work * 2 ** 64), false);
});
