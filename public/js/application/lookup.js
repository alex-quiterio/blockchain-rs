// Looking things up for the explorer: the latest blocks, any block, transaction or address, and
// where the network stands. It reads only; the explorer re-checks what comes back.

import { ports } from "./ports.js";

/** Where the network stands: its rules, and its tip. */
export const context = { consensus: null, tip: null, height: -1 };

/** Brings `context` up to date. The rules are fetched only when the tip moves. Returns whether it moved. */
export async function refreshContext() {
  const status = await ports.node.status();
  const moved = status.tip !== context.tip;
  if (moved) context.consensus = await ports.node.consensus();
  Object.assign(context, { tip: status.tip, height: status.height });
  return moved;
}

/** The latest `count` blocks, the network's status, and what's waiting. */
export async function latest(count) {
  const status = await ports.node.status();
  const from = Math.max(0, status.height - count + 1);
  const [blocks, pending] = await Promise.all([ports.node.blocks(from), ports.node.pending()]);
  return { status, blocks, pending };
}

/** A block by height or hash, with the one before it and, under proof of work, the work it needed. */
export async function block(id) {
  const found = await ports.node.block(id);
  const prev = found.index ? await ports.node.block(found.index - 1) : null;
  const [work] = found.validator ? [] : (await ports.node.works(found.index, 1)).map(Number);
  return { block: found, prev, work };
}

/** A transaction by signature, where it is, its block, and, once confirmed, the proof it's in it. */
export async function transaction(signature) {
  const found = await ports.node.transaction(signature);
  const placed = found.block;
  const [inBlock, proof] = placed
    ? await Promise.all([ports.node.block(placed.height), ports.node.proof(signature)])
    : [null, null];
  return { transaction: found.transaction, placed, block: inBlock, proof };
}

/** An address's account and its history. */
export async function address(addr) {
  const [account, history] = await Promise.all([ports.node.balance(addr), ports.node.history(addr)]);
  return { account, history };
}

const HEX = /^[0-9a-f]+$/i;

/** What a search names: a block height, a transaction, or a 64-hex block hash or address. */
export async function search(query) {
  const q = query.trim();
  if (/^\d+$/.test(q)) return { kind: "block", id: q };
  if (HEX.test(q) && q.length === 128) return { kind: "tx", id: q };
  if (HEX.test(q) && q.length === 64) {
    return ports.node.block(q).then(() => ({ kind: "block", id: q }), () => ({ kind: "address", id: q }));
  }
  throw new Error("Try a block height, a 64-character block hash or address, or a 128-character signature.");
}
