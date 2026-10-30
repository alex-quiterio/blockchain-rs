// The node as the wallet page last saw it: its latest blocks, what's waiting, its rules, its
// peers, and what each wallet holds. Only the latest blocks are kept, so the page costs the same
// however long the chain gets; the explorer shows the rest. A read model: it's brought up to
// date from the node it's given, and tells whoever subscribed each time it changes.

import type { Block } from "@domain/blocks";
import type { Transfer } from "@domain/transfers";
import type { Wallets } from "@domain/wallets/wallets";
import type {
  Account,
  Consensus,
  Fees,
  NodeInfo,
  NodePort,
  Peer,
  Status,
} from "./ports.ts";

export const SHOWN_BLOCKS = 30;

export interface NodeState {
  height: number;
  /** The latest few blocks, oldest first. */
  blocks: Block[];
  /** The work each kept block required, by height. */
  works: Record<number, number>;
  pending: Transfer[];
  valid: boolean | null;
  problem: string | null;
  consensus: Consensus | null;
  fees: Fees | null;
  accounts: Record<string, Account>;
  peers: Peer[];
  node: NodeInfo | null;
}

type Listener = () => void;

export class NodeView {
  #node: NodePort;
  #wallets: Wallets;
  #state: NodeState = {
    height: -1,
    blocks: [],
    works: {},
    pending: [],
    valid: null,
    problem: null,
    consensus: null,
    fees: null,
    accounts: {},
    peers: [],
    node: null,
  };
  #listeners = new Set<Listener>();
  #peerListeners = new Set<Listener>();

  constructor(node: NodePort, wallets: Wallets) {
    this.#node = node;
    this.#wallets = wallets;
  }

  /** The view as of the last refresh. Only the view changes it. */
  get state(): Readonly<NodeState> {
    return this.#state;
  }

  /** Calls `listener` after every refresh. Returns how to stop. */
  onChange(listener: Listener): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  /** Calls `listener` whenever the peers alone are brought up to date. Returns how to stop. */
  onPeersChange(listener: Listener): () => void {
    this.#peerListeners.add(listener);
    return () => this.#peerListeners.delete(listener);
  }

  /** The rules, once the first refresh has fetched them. */
  consensus(): Consensus {
    if (!this.#state.consensus)
      throw new Error("The node's rules haven't been fetched yet");
    return this.#state.consensus;
  }
  isStake(): boolean {
    return this.#state.consensus?.kind === "proof_of_stake";
  }
  tip(): string | undefined {
    return this.#state.blocks.at(-1)?.hash;
  }
  /** Fees only move coins around; the supply grows by the block reward alone. */
  supply(): number {
    return (
      Math.max(0, this.#state.height) * (this.#state.consensus?.reward ?? 0)
    );
  }
  /** The block at a height, if it's among the ones kept. */
  blockAt(height: number): Block | undefined {
    return this.#state.blocks[height - (this.#state.blocks[0]?.index ?? 0)];
  }
  /** Enough blocks to draw the shown ones, the block before each, and a retarget window back. */
  #kept(): number {
    return SHOWN_BLOCKS + 1 + (this.#state.consensus?.retarget_interval ?? 0);
  }

  /** Brings everything up to date with the node, then tells the subscribers. */
  async refresh(): Promise<void> {
    const node = this.#node;
    const [status, pending, rules, peers, info, fees] = await Promise.all([
      node.status(),
      node.pending(),
      node.consensus(),
      node.peers(),
      this.#state.node ?? node.info(),
      node.fees(),
    ]);
    Object.assign(this.#state, {
      pending,
      consensus: rules,
      peers,
      node: info,
      fees,
    });
    const forked = await this.#refreshBlocks(status);
    // A full replay of the chain: once on opening, and again only if a fork replaced our tip.
    if (this.#state.valid === null || forked) {
      const validity = await node.validity();
      this.#state.valid = validity.valid;
      this.#state.problem = validity.problem;
    }
    const accounts = await Promise.all(
      this.#wallets.list.map((w) => node.balance(w.address)),
    );
    this.#state.accounts = Object.fromEntries(
      accounts.map((a) => [a.address, a]),
    );
    tell(this.#listeners);
  }

  /**
   * Brings the kept blocks up to the node's tip. Usually that's fetching just the new ones; if
   * they don't link to our tip, a fork replaced it, and the latest few are fetched again. Returns
   * whether that happened.
   */
  async #refreshBlocks(status: Status): Promise<boolean> {
    if (status.tip === this.tip()) return false;
    const state = this.#state;
    const kept = this.#kept();
    const have = state.blocks.at(-1)?.index ?? -1;
    const oldest = Math.max(0, status.height + 1 - kept);
    let blocks: Block[] | null = null;
    if (have >= 0 && status.height > have && status.height - have < kept) {
      const fresh = await this.#node.blocks(have + 1);
      if (fresh[0]?.prev_hash === this.tip())
        blocks = state.blocks.concat(fresh).slice(-kept);
    }
    const forked = blocks === null && have >= 0 && status.height - have < kept;
    blocks ??= await this.#node.blocks(oldest);
    const start = blocks[0]?.index ?? 0;
    // Per-block required work; past 2^53 it's approximate, which is fine for display.
    const works = this.isStake()
      ? []
      : await this.#node.works(start, blocks.length);
    Object.assign(state, {
      height: status.height,
      blocks,
      works: Object.fromEntries(works.map((w, i) => [start + i, Number(w)])),
    });
    return forked;
  }

  /**
   * Whether the node moved on since the last refresh: a new tip, or a different number waiting.
   * Cheap enough to ask every few seconds. If it didn't, the peers are brought up to date anyway,
   * and their subscribers told.
   */
  async hasMoved(): Promise<boolean> {
    const [status, peers] = await Promise.all([
      this.#node.status(),
      this.#node.peers(),
    ]);
    if (
      status.tip !== this.tip() ||
      status.pending !== this.#state.pending.length
    )
      return true;
    this.#state.peers = peers;
    tell(this.#peerListeners);
    return false;
  }
}

/**
 * Calls every listener, even after one throws, so one part failing to draw doesn't stop the
 * rest; then throws the first failure, for the caller to show.
 */
function tell(listeners: Iterable<Listener>): void {
  const failures: unknown[] = [];
  for (const listener of listeners) {
    try {
      listener();
    } catch (err) {
      failures.push(err);
    }
  }
  if (failures.length) throw failures[0];
}
