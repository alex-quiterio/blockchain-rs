import assert from "node:assert/strict";
import { test } from "node:test";
import { sha256Hex } from "@domain/encoding";
import { LENGTH, commitment, reveal } from "@domain/wallets/reveals";

const secret = "11".repeat(32);

test("each reveal hashes to the link shown before it, back to the commitment", async () => {
  const committed = await commitment(secret, 0);
  const first = await reveal(secret, 0, LENGTH);
  const second = await reveal(secret, 0, LENGTH - 1);
  assert.equal(
    await sha256Hex(first),
    committed,
    "the first reveal opens the commitment",
  );
  assert.equal(
    await sha256Hex(second),
    first,
    "and each next one opens the one before",
  );
});

test("each epoch's chain is different, and the same key always gives the same one", async () => {
  assert.notEqual(await commitment(secret, 0), await commitment(secret, 1));
  assert.equal(await commitment(secret, 0), await commitment(secret, 0));
});
