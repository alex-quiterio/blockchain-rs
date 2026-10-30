// Looking things up for the explorer: the latest blocks, any block, transaction or address, and
// where the network stands. It reads only; the explorer re-checks what comes back.

import type { Block } from "@domain/blocks";
import type { Transfer } from "@domain/transfers";
import type {
  Account,
  Consensus,
  Located,
  NodePort,
  Placement,
  Proof,
  Status,
} from "./ports.ts";

/** Where the network stands: its rules, and its tip. */
export interface Context {
  consensus: Consensus | null;
  tip: string | null;
  height: number;
}

export interface FoundTransaction {
  transaction: Transfer;
  placed: Placement | null;
  block: Block | null;
  proof: Proof | null;
}

export type Found = { kind: "block" | "tx" | "address"; id: string };

const HEX = /^[0-9a-f]+$/i;

/** The explorer's reads, from the node it's given: an application service. */
export class Lookup {
  #node: NodePort;
  #context: Context = { consensus: null, tip: null, height: -1 };

  constructor(node: NodePort) {
    this.#node = node;
  }

  /** Where the network stood at the last refresh. */
  get context(): Readonly<Context> {
    return this.#context;
  }

  /** The rules, once the first refresh has fetched them. */
  rules(): Consensus {
    if (!this.#context.consensus)
      throw new Error("The node's rules haven't been fetched yet");
    return this.#context.consensus;
  }

  /**
   * Brings the context up to date. The rules are fetched only when the tip moves. Returns whether
   * it moved.
   */
  async refreshContext(): Promise<boolean> {
    const status = await this.#node.status();
    const moved = status.tip !== this.#context.tip;
    const consensus = moved
      ? await this.#node.consensus()
      : this.#context.consensus;
    this.#context = { consensus, tip: status.tip, height: status.height };
    return moved;
  }

  /** The latest `count` blocks, the network's status, and what's waiting. */
  async latest(
    count: number,
  ): Promise<{ status: Status; blocks: Block[]; pending: Transfer[] }> {
    const status = await this.#node.status();
    const from = Math.max(0, status.height - count + 1);
    const [blocks, pending] = await Promise.all([
      this.#node.blocks(from),
      this.#node.pending(),
    ]);
    return { status, blocks, pending };
  }

  /**
   * A block by height or hash, with the one before it and, under proof of work, the work it
   * needed.
   */
  async block(
    id: string,
  ): Promise<{ block: Block; prev: Block | null; work: number | undefined }> {
    const found = await this.#node.block(id);
    const prev = found.index ? await this.#node.block(found.index - 1) : null;
    const [work] = found.validator
      ? []
      : (await this.#node.works(found.index, 1)).map(Number);
    return { block: found, prev, work };
  }

  /**
   * A transaction by signature, where it is, its block, and, once confirmed, the proof it's in
   * it.
   */
  async transaction(signature: string): Promise<FoundTransaction> {
    const found = await this.#node.transaction(signature);
    const placed = found.block;
    const [inBlock, proof] = placed
      ? await Promise.all([
          this.#node.block(placed.height),
          this.#node.proof(signature),
        ])
      : [null, null];
    return { transaction: found.transaction, placed, block: inBlock, proof };
  }

  /** An address's account and its history. */
  async address(
    addr: string,
  ): Promise<{ account: Account; history: Located[] }> {
    const [account, history] = await Promise.all([
      this.#node.balance(addr),
      this.#node.history(addr),
    ]);
    return { account, history };
  }

  /** What a search names: a block height, a transaction, or a 64-hex block hash or address. */
  async search(query: string): Promise<Found> {
    const q = query.trim();
    if (/^\d+$/.test(q)) return { kind: "block", id: q };
    if (HEX.test(q) && q.length === 128) return { kind: "tx", id: q };
    if (HEX.test(q) && q.length === 64) {
      return this.#node.block(q).then(
        (): Found => ({ kind: "block", id: q }),
        (): Found => ({ kind: "address", id: q }),
      );
    }
    throw new Error(
      "Try a block height, a 64-character block hash or address, or a 128-character signature.",
    );
  }
}
