// Looking things up for the explorer: the latest blocks, any block, transaction or address, and
// where the network stands. It reads only; the explorer re-checks what comes back.

import type { Block } from "@domain/blocks";
import type { Transfer } from "@domain/transfers";
import {
  ports,
  type Account,
  type Consensus,
  type Located,
  type Placement,
  type Proof,
  type Status,
} from "./ports.ts";

/** Where the network stands: its rules, and its tip. */
export const context: {
  consensus: Consensus | null;
  tip: string | null;
  height: number;
} = {
  consensus: null,
  tip: null,
  height: -1,
};

/** The rules, once the first refresh has fetched them. */
export const rules = (): Consensus => {
  if (!context.consensus)
    throw new Error("The node's rules haven't been fetched yet");
  return context.consensus;
};

/**
 * Brings `context` up to date. The rules are fetched only when the tip moves. Returns whether it
 * moved.
 */
export async function refreshContext(): Promise<boolean> {
  const status = await ports.node.status();
  const moved = status.tip !== context.tip;
  if (moved) context.consensus = await ports.node.consensus();
  Object.assign(context, { tip: status.tip, height: status.height });
  return moved;
}

/** The latest `count` blocks, the network's status, and what's waiting. */
export async function latest(
  count: number,
): Promise<{ status: Status; blocks: Block[]; pending: Transfer[] }> {
  const status = await ports.node.status();
  const from = Math.max(0, status.height - count + 1);
  const [blocks, pending] = await Promise.all([
    ports.node.blocks(from),
    ports.node.pending(),
  ]);
  return { status, blocks, pending };
}

/**
 * A block by height or hash, with the one before it and, under proof of work, the work it needed.
 */
export async function block(
  id: string,
): Promise<{ block: Block; prev: Block | null; work: number | undefined }> {
  const found = await ports.node.block(id);
  const prev = found.index ? await ports.node.block(found.index - 1) : null;
  const [work] = found.validator
    ? []
    : (await ports.node.works(found.index, 1)).map(Number);
  return { block: found, prev, work };
}

export interface FoundTransaction {
  transaction: Transfer;
  placed: Placement | null;
  block: Block | null;
  proof: Proof | null;
}

/**
 * A transaction by signature, where it is, its block, and, once confirmed, the proof it's in it.
 */
export async function transaction(
  signature: string,
): Promise<FoundTransaction> {
  const found = await ports.node.transaction(signature);
  const placed = found.block;
  const [inBlock, proof] = placed
    ? await Promise.all([
        ports.node.block(placed.height),
        ports.node.proof(signature),
      ])
    : [null, null];
  return { transaction: found.transaction, placed, block: inBlock, proof };
}

/** An address's account and its history. */
export async function address(
  addr: string,
): Promise<{ account: Account; history: Located[] }> {
  const [account, history] = await Promise.all([
    ports.node.balance(addr),
    ports.node.history(addr),
  ]);
  return { account, history };
}

export type Found = { kind: "block" | "tx" | "address"; id: string };

const HEX = /^[0-9a-f]+$/i;

/** What a search names: a block height, a transaction, or a 64-hex block hash or address. */
export async function search(query: string): Promise<Found> {
  const q = query.trim();
  if (/^\d+$/.test(q)) return { kind: "block", id: q };
  if (HEX.test(q) && q.length === 128) return { kind: "tx", id: q };
  if (HEX.test(q) && q.length === 64) {
    return ports.node.block(q).then(
      (): Found => ({ kind: "block", id: q }),
      (): Found => ({ kind: "address", id: q }),
    );
  }
  throw new Error(
    "Try a block height, a 64-character block hash or address, or a 128-character signature.",
  );
}
