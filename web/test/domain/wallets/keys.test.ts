import assert from "node:assert/strict";
import { test } from "node:test";
import { isSignedBy, transferIsSigned } from "@domain/verification/signatures";
import { generateKeypair, sign, signTransfer } from "@domain/wallets/keys";

test("a new keypair is a 32-byte public key and a 32-byte secret, as hex", async () => {
  const { address, secret_key } = await generateKeypair();
  assert.match(address, /^[0-9a-f]{64}$/);
  assert.match(secret_key, /^[0-9a-f]{64}$/);
});

test("what a key signs is checked against its address, and nothing else", async () => {
  const keys = await generateKeypair();
  const other = await generateKeypair();
  const signature = await sign(keys.secret_key, "hello");
  assert.ok(await isSignedBy(keys.address, "hello", signature));
  assert.equal(await isSignedBy(keys.address, "hullo", signature), false);
  assert.equal(await isSignedBy(other.address, "hello", signature), false);
});

test("a signed transfer leaves out a zero fee, and changing it breaks the signature", async () => {
  const keys = await generateKeypair();
  const tx = await signTransfer(keys.secret_key, {
    from: keys.address,
    to: "bb".repeat(32),
    amount: 5,
    nonce: 0,
    fee: 0,
  });
  assert.equal("fee" in tx, false);
  assert.ok(await transferIsSigned(tx));
  assert.equal(await transferIsSigned({ ...tx, amount: 500 }), false);
});
