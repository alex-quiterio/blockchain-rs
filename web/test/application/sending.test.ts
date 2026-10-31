import assert from "node:assert/strict";
import { test } from "node:test";
import { send } from "@application/sending";
import { createWallet } from "@application/wallets";
import { STAKE, type Transfer } from "@domain/transfers";
import { sha256Hex } from "@domain/encoding";
import { transferIsSigned } from "@domain/verification/signatures";
import { reveal, LENGTH } from "@domain/wallets/reveals";
import { Wallets } from "@domain/wallets/wallets";
import { account, emptyRepository, fakeNode } from "./fake_node.ts";

const genesis = "0a".repeat(32);
const fake = fakeNode({
  balance: (address) =>
    account({ address: address as string, next_nonce: 4, commitment_due: 0 }),
  submitTransaction: (tx) => tx,
  status: () => ({
    chain: genesis,
    height: 0,
    tip: genesis,
    weight: "1",
    pending: 0,
  }),
});
const wallets = new Wallets(emptyRepository()).load();
await wallets.setUp("correct horse");
await createWallet(wallets, "Alice");
const alice = wallets.list[0]!;

test("a transfer is signed here, with the sender's next nonce, and submitted", async () => {
  const tx = await send(fake.node, wallets, {
    from: alice,
    to: "bb".repeat(32),
    amount: 5,
    fee: 2,
  });
  const [submitted] = fake.called("submitTransaction").at(-1)!.args as [
    Transfer,
  ];
  assert.deepEqual(submitted, tx);
  assert.equal(tx.nonce, 4, "the node's next nonce for the sender");
  assert.equal(tx.fee, 2);
  assert.equal(tx.commitment, undefined, "only a first stake commits");
  assert.equal(tx.chain, genesis, "it names the node's chain");
  assert.ok(await transferIsSigned(tx), "the signature covers the chain");
});

test("a first stake commits to the reveal chain the wallet will forge with", async () => {
  const tx = await send(fake.node, wallets, {
    from: alice,
    to: STAKE,
    amount: 10,
    fee: 0,
  });
  const first = await reveal(wallets.secretOf(alice.address), 0, LENGTH);
  assert.equal(
    tx.commitment,
    await sha256Hex(first),
    "its first reveal will open the commitment",
  );
  assert.ok(await transferIsSigned(tx), "the signature covers the commitment");
});

test("nothing is sent while the wallets are locked", async () => {
  wallets.lock();
  const before = fake.calls.length;
  await assert.rejects(
    send(fake.node, wallets, {
      from: alice,
      to: "bb".repeat(32),
      amount: 1,
      fee: 0,
    }),
    /Unlock/,
  );
  assert.equal(fake.calls.length, before, "the node isn't even asked");
});
