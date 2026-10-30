// What the application talks through, wired in by each page's main.ts, so the use cases never
// name an adapter: `node`, the node's API, and `walletsRepository`, where the wallets are kept.
// The node's port is declared here, with what each of its answers holds.

import type { Block, BlockHeader, ProofStep } from "@domain/blocks";
import type { Transfer } from "@domain/transfers";
import type { WalletsRepository } from "@domain/wallets/wallets";

export interface Status {
  height: number;
  tip: string;
  weight: string;
  pending: number;
}
export interface NodeInfo {
  version: string;
  commit: string;
  started_at: string;
}
export interface Validity {
  valid: boolean;
  problem: string | null;
  length: number;
  weight: string;
}

export interface Consensus {
  kind: "proof_of_work" | "proof_of_stake";
  difficulty: number | null;
  reward: number;
  max_block_transfers: number | null;
  next_validator: string | null;
  stakes: { address: string; amount: number }[];
  total_stake: number;
  beacon: string | null;
  work: string | null;
  target_block_secs: number | null;
  retarget_interval: number | null;
  next_retarget: number | null;
}

export interface Account {
  address: string;
  balance: number;
  staked: number;
  next_nonce: number;
  next_epoch: number;
  reveals_left: number | null;
  commitment_due: number | null;
}

export interface Fees {
  fee: number;
  room: number | null;
  capacity: number | null;
  waiting: number;
  recent: {
    height: number;
    transfers: number;
    full: boolean;
    lowest_fee: number | null;
  }[];
}

export interface Placement {
  height: number;
  hash: string;
  position: number;
  timestamp: string;
  confirmations: number;
}
export interface Located {
  transaction: Transfer;
  status: "confirmed" | "pending";
  block: Placement | null;
}
export interface Proof {
  transaction: Transfer;
  leaf: string;
  proof: ProofStep[];
  header: BlockHeader;
  confirmations: number;
}
export interface Peer {
  url: string;
  height: number | null;
  last_seen: string | null;
  error: string | null;
}
export interface SyncReport {
  adopted_from: string | null;
  reached: number;
  length: number;
}

/** The node, as the application talks to it. */
export interface NodePort {
  status(): Promise<Status>;
  info(): Promise<NodeInfo>;
  consensus(): Promise<Consensus>;
  validity(): Promise<Validity>;
  blocks(from: number, limit?: number): Promise<Block[]>;
  works(from: number, limit: number): Promise<string[]>;
  block(heightOrHash: string | number): Promise<Block>;
  pending(): Promise<Transfer[]>;
  fees(): Promise<Fees>;
  transaction(signature: string): Promise<Located>;
  proof(signature: string): Promise<Proof>;
  balance(address: string): Promise<Account>;
  history(address: string): Promise<Located[]>;
  peers(): Promise<Peer[]>;
  submitTransaction(tx: Transfer): Promise<Transfer>;
  mine(miner: string): Promise<Block>;
  propose(validator: string, reveal: string | undefined): Promise<Block>;
  submitBlock(block: Block): Promise<Block>;
  addPeer(url: string): Promise<{ url: string; added: boolean }>;
  sync(): Promise<SyncReport>;
}

export interface Ports {
  node: NodePort;
  walletsRepository: WalletsRepository;
}

const wired: Partial<Ports> = {};

/** Wires the adapters in. Each page does this once, before anything else runs. */
export function wire(adapters: Partial<Ports>): void {
  Object.assign(wired, adapters);
}

/** The adapters, as wired. Asking for one that wasn't is a mistake in the page, so it throws. */
export const ports = {
  get node(): NodePort {
    return wired.node ?? missing("node");
  },
  get walletsRepository(): WalletsRepository {
    return wired.walletsRepository ?? missing("walletsRepository");
  },
};

function missing(port: keyof Ports): never {
  throw new Error(`No adapter wired for ${port}`);
}
