// The node as the wallet page last saw it: its latest blocks, what's waiting, its rules, its
// peers, and what each wallet holds. Only the latest blocks are kept, so the page costs the same
// however long the chain gets; the explorer shows the rest.

import { ports } from "./ports.js";
import { wallets } from "./wallets.js";

export const SHOWN_BLOCKS = 30;

// `blocks` holds the latest few, oldest first; `works` is the required work, by height.
export const state = { height: -1, blocks: [], works: {}, pending: [], valid: null, problem: null, consensus: null, fees: null, accounts: {}, peers: [], node: null };

export const isStake = () => state.consensus?.kind === "proof_of_stake";
export const tip = () => state.blocks.at(-1)?.hash;
// Fees only move coins around; the supply grows by the block reward alone.
export const supply = () => Math.max(0, state.height) * (state.consensus?.reward ?? 0);
/** The block at a height, if it's among the ones kept. */
export const blockAt = (height) => state.blocks[height - (state.blocks[0]?.index ?? 0)];
/** Enough blocks to draw the shown ones, the block before each, and a retarget window back. */
const kept = () => SHOWN_BLOCKS + 1 + (state.consensus?.retarget_interval ?? 0);

/** Brings everything up to date with the node. */
export async function refreshView() {
  const node = ports.node;
  const [status, pending, consensus, peers, info, fees] = await Promise.all([
    node.status(), node.pending(), node.consensus(), node.peers(), state.node || node.info(), node.fees(),
  ]);
  Object.assign(state, { pending, consensus, peers, node: info, fees });
  const forked = await refreshBlocks(status);
  // A full replay of the chain: once on opening, and again only if a fork replaced our tip.
  if (state.valid === null || forked) {
    const validity = await node.validity();
    state.valid = validity.valid;
    state.problem = validity.problem;
  }
  const accounts = await Promise.all(wallets().list.map((w) => node.balance(w.address)));
  state.accounts = Object.fromEntries(accounts.map((a) => [a.address, a]));
}

/**
 * Brings the kept blocks up to the node's tip. Usually that's fetching just the new ones; if they
 * don't link to our tip, a fork replaced it, and the latest few are fetched again. Returns
 * whether that happened.
 */
async function refreshBlocks(status) {
  if (status.tip === tip()) return false;
  const have = state.blocks.at(-1)?.index ?? -1;
  const oldest = Math.max(0, status.height + 1 - kept());
  let blocks = null;
  if (have >= 0 && status.height > have && status.height - have < kept()) {
    const fresh = await ports.node.blocks(have + 1);
    if (fresh[0]?.prev_hash === tip()) blocks = state.blocks.concat(fresh).slice(-kept());
  }
  const forked = blocks === null && have >= 0 && status.height - have < kept();
  blocks ??= await ports.node.blocks(oldest);
  const start = blocks[0]?.index ?? 0;
  // Per-block required work; past 2^53 it's approximate, which is fine for display.
  const works = isStake() ? [] : await ports.node.works(start, blocks.length);
  Object.assign(state, {
    height: status.height,
    blocks,
    works: Object.fromEntries(works.map((w, i) => [start + i, Number(w)])),
  });
  return forked;
}

/**
 * Whether the node moved on since the last refresh: a new tip, or a different number waiting.
 * Cheap enough to ask every few seconds. If it didn't, the peers are brought up to date anyway.
 */
export async function hasMoved() {
  const [status, peers] = await Promise.all([ports.node.status(), ports.node.peers()]);
  if (status.tip !== tip() || status.pending !== state.pending.length) return true;
  state.peers = peers;
  return false;
}
