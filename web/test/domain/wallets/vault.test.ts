import assert from "node:assert/strict";
import { test } from "node:test";
import { ITERATIONS, deriveKey, open, seal } from "@domain/wallets/vault";

async function session(passphrase: string) {
  const salt = crypto.getRandomValues(new Uint8Array(16));
  return {
    salt,
    iterations: ITERATIONS,
    key: await deriveKey(passphrase, salt, ITERATIONS),
  };
}

test("a sealed vault opens with its passphrase, and only with it", async () => {
  const secrets = { alice: "11".repeat(32) };
  const sealed = await seal(secrets, await session("correct horse"));
  assert.equal(
    sealed.data.includes("11".repeat(32)),
    false,
    "nothing is saved in the clear",
  );
  assert.deepEqual((await open(sealed, "correct horse")).secrets, secrets);
  await assert.rejects(open(sealed, "wrong horse"), /Wrong passphrase/);
});

test("sealing twice uses a fresh IV, so the same keys never look the same", async () => {
  const keys = await session("correct horse");
  const [a, b] = [await seal({ x: "1" }, keys), await seal({ x: "1" }, keys)];
  assert.notEqual(a.iv, b.iv);
  assert.notEqual(a.data, b.data);
});
