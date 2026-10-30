import assert from "node:assert/strict";
import { test } from "node:test";
import {
  blockIsSigned,
  transferIsSigned,
} from "@domain/verification/signatures";
import { forged, mined } from "../../fixtures.ts";

const transfers = mined.block.transactions.slice(1);

test("every transfer the node took is signed by its sender", async () => {
  for (const tx of transfers) assert.ok(await transferIsSigned(tx));
});

test("changing a signed transfer's amount, fee or recipient breaks its signature", async () => {
  const tx = transfers.find((t) => t.fee)!;
  assert.equal(await transferIsSigned({ ...tx, amount: tx.amount + 1 }), false);
  assert.equal(await transferIsSigned({ ...tx, fee: 0 }), false);
  assert.equal(await transferIsSigned({ ...tx, to: transfers[0]!.to }), false);
});

test("a forged block is signed by its validator; a mined one isn't signed at all", async () => {
  assert.ok(await blockIsSigned(forged));
  assert.equal(
    await blockIsSigned({ ...forged, hash: mined.block.hash }),
    false,
  );
  assert.equal(await blockIsSigned(mined.block), false);
});
