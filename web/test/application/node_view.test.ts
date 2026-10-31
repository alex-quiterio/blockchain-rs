import assert from "node:assert/strict";
import { test } from "node:test";
import { NodeView, SHOWN_BLOCKS } from "@application/node_view";
import type { Consensus, Peer } from "@application/ports";
import { createWallet } from "@application/wallets";
import type { Block } from "@domain/blocks";
import { Wallets } from "@domain/wallets/wallets";
import { account, emptyRepository, fakeNode } from "./fake_node.ts";

/** How many blocks the view keeps: the shown ones and the one before them, with no retargeting. */
const KEPT = SHOWN_BLOCKS + 1;

/** `count` more blocks on top of `chain`, each naming the one before it; `fork` sets them apart. */
function grow(chain: Block[], count: number, fork = "main"): Block[] {
  const grown = [...chain];
  for (let i = 0; i < count; i++) {
    const index = grown.length;
    grown.push({
      index,
      timestamp: new Date(index * 1000).toISOString(),
      prev_hash: grown.at(-1)?.hash ?? "00".repeat(32),
      merkle_root: "11".repeat(32),
      nonce: 0,
      hash: `${fork}-${index}`,
      transactions: [],
    });
  }
  return grown;
}

/** A node serving whatever `chain` holds at the moment, and the view of it. */
async function watching(start: number) {
  let chain = grow([], start);
  let peers: Peer[] = [];
  const fake = fakeNode({
    status: () => ({
      chain: chain[0]!.hash,
      height: chain.length - 1,
      tip: chain.at(-1)!.hash,
      weight: "1",
      pending: 0,
    }),
    pending: () => [],
    consensus: () =>
      ({
        kind: "proof_of_work",
        reward: 50,
        work: "16",
        retarget_interval: null,
      }) as Consensus,
    peers: () => peers,
    info: () => ({ version: "0.0.0", commit: "abc", started_at: "" }),
    fees: () => ({
      fee: 0,
      room: null,
      capacity: null,
      waiting: 0,
      recent: [],
    }),
    validity: () => ({
      valid: true,
      problem: null,
      length: chain.length,
      weight: "1",
    }),
    blocks: (from, limit) =>
      chain.slice(from, limit == null ? undefined : from + limit),
    works: (_from, limit) => Array<string>(limit).fill("16"),
    balance: (address) => account({ address: address as string }),
  });
  const wallets = new Wallets(emptyRepository()).load();
  await wallets.setUp("correct horse");
  await createWallet(wallets, "Alice");
  const view = new NodeView(fake.node, wallets);
  return {
    view,
    wallets,
    fake,
    /** Changes what the node serves. */
    serve: (next: (chain: Block[]) => Block[]) => {
      chain = next(chain);
    },
    setPeers: (next: Peer[]) => {
      peers = next;
    },
    /** The `from` of each request for blocks since `since` calls. */
    fetchedFrom: (since: number) =>
      fake
        .called("blocks")
        .slice(since)
        .map((c) => c.args[0]),
  };
}

const indexes = (view: NodeView) => [
  view.state.blocks[0]?.index,
  view.state.blocks.at(-1)?.index,
];

test("opening keeps only the latest blocks, replays the chain once, and tells its subscribers", async () => {
  const { view, wallets, fake } = await watching(40);
  let told = 0;
  view.onChange(() => told++);
  await view.refresh();

  assert.deepEqual(indexes(view), [40 - KEPT, 39]);
  assert.equal(view.state.blocks.length, KEPT);
  assert.equal(view.state.height, 39);
  assert.equal(view.state.works[39], 16, "each kept block's work");
  assert.equal(view.state.valid, true);
  assert.equal(fake.called("validity").length, 1);
  assert.ok(
    view.state.accounts[wallets.list[0]!.address],
    "each wallet's account",
  );
  assert.equal(told, 1);
});

test("a few new blocks are fetched alone, without replaying the chain", async () => {
  const { view, fake, serve, fetchedFrom } = await watching(40);
  await view.refresh();
  const before = fake.called("blocks").length;
  serve((chain) => grow(chain, 2));
  await view.refresh();

  assert.deepEqual(
    fetchedFrom(before),
    [40],
    "only from the block after our tip",
  );
  assert.deepEqual(indexes(view), [42 - KEPT, 41]);
  assert.equal(view.state.blocks.length, KEPT, "still only the latest few");
  assert.equal(fake.called("validity").length, 1, "no replay");
});

test("new blocks that don't link to our tip mean a fork replaced it: the latest are fetched again and the chain replayed", async () => {
  const { view, fake, serve, fetchedFrom } = await watching(40);
  await view.refresh();
  const before = fake.called("blocks").length;
  // The last three blocks replaced by a heavier branch one block longer.
  serve((chain) => grow(chain.slice(0, 37), 4, "fork"));
  await view.refresh();

  assert.deepEqual(
    fetchedFrom(before),
    [40, 41 - KEPT],
    "the new ones, then the latest again",
  );
  assert.equal(view.tip(), "fork-40");
  assert.equal(
    view.blockAt(37)?.hash,
    "fork-37",
    "the replaced blocks are gone",
  );
  assert.equal(view.blockAt(36)?.hash, "main-36", "the shared ones are kept");
  assert.equal(fake.called("validity").length, 2, "replayed after the fork");
});

test("a jump past what's kept fetches the latest afresh, without calling it a fork", async () => {
  const { view, fake, serve, fetchedFrom } = await watching(40);
  await view.refresh();
  const before = fake.called("blocks").length;
  serve((chain) => grow(chain, 50));
  await view.refresh();

  assert.deepEqual(fetchedFrom(before), [90 - KEPT]);
  assert.deepEqual(indexes(view), [90 - KEPT, 89]);
  assert.equal(fake.called("validity").length, 1, "no replay");
});

test("polling says whether the node moved; if not, it brings the peers up to date and tells their subscribers alone", async () => {
  const { view, serve, setPeers } = await watching(5);
  await view.refresh();
  let changes = 0;
  let peerChanges = 0;
  view.onChange(() => changes++);
  view.onPeersChange(() => peerChanges++);

  const peer = {
    url: "http://b",
    height: 4,
    last_seen: null,
    error: null,
    banned_until: null,
  };
  setPeers([peer]);
  assert.equal(await view.hasMoved(), false);
  assert.deepEqual(view.state.peers, [peer]);
  assert.deepEqual([changes, peerChanges], [0, 1]);

  serve((chain) => grow(chain, 1));
  assert.equal(await view.hasMoved(), true);
});

test("one subscriber failing doesn't stop the others; the refresh then fails with it", async () => {
  const { view } = await watching(5);
  let reached = false;
  view.onChange(() => {
    throw new Error("a part couldn't draw");
  });
  view.onChange(() => {
    reached = true;
  });
  await assert.rejects(view.refresh(), /a part couldn't draw/);
  assert.ok(reached);
});
