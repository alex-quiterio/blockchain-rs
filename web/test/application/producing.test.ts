import assert from "node:assert/strict";
import { test } from "node:test";
import { wire } from "@application/ports";
import { forge, mine } from "@application/producing";
import { createWallet, wallets } from "@application/wallets";
import type { Block } from "@domain/blocks";
import { isSignedBy } from "@domain/verification/signatures";
import { reveal } from "@domain/wallets/reveals";
import { forged } from "../fixtures.ts";
import { account, emptyRepository, fakeNode } from "./fake_node.ts";

const proposal = (): Block => ({ ...forged, signature: undefined });
const fake = fakeNode({
  balance: (address) =>
    account({ address: address as string, next_epoch: 1, reveals_left: 997 }),
  propose: () => proposal(),
  submitBlock: (block) => block,
  mine: () => forged,
});
wire({ node: fake.node, walletsRepository: emptyRepository() });
await wallets().setUp("correct horse");
await createWallet("Val");
const val = wallets().list[0]!;

test("forging signs the proposed block's hash here; only the signature goes back", async () => {
  const block = await forge(val, false);
  assert.ok(await isSignedBy(val.address, block.hash, block.signature!));
  const [submitted] = fake.called("submitBlock").at(-1)!.args as [Block];
  assert.equal(submitted.signature, block.signature);
  assert.deepEqual(
    fake.called("propose").at(-1)!.args,
    [val.address, undefined],
    "no reveal while anyone may forge",
  );
});

test("once validators are drawn, forging reveals the next link of the wallet's chain", async () => {
  await forge(val, true);
  const expected = await reveal(wallets().secretOf(val.address), 0, 997);
  assert.deepEqual(fake.called("propose").at(-1)!.args, [
    val.address,
    expected,
  ]);
});

test("mining asks the node, paying the miner", async () => {
  await mine(val.address);
  assert.deepEqual(fake.called("mine").at(-1)!.args, [val.address]);
});
