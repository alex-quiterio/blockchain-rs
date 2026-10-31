// What the application talks to the node through: its port, declared here with what each of its
// answers holds. Each page's main.ts hands an adapter for it to the use cases that need it, so
// they never name one. Where the wallets are kept is the domain's port, WalletsRepository.

import type { Block, BlockHeader, ProofStep } from "@domain/blocks";
import type { Transfer } from "@domain/transfers";

export interface Status {
  /** The chain every transfer must name: its genesis hash. */
  chain: string;
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
  /** Until when the node ignores it for sending invalid chains. */
  banned_until: string | null;
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
