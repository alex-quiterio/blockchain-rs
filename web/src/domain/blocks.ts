// What a block is to this page: its header, the transfers it carries, and the steps of a Merkle
// proof that one of them is in it. Value objects, as the node sends them.

import type { Transfer } from "./transfers.ts";

export interface BlockHeader {
  index: number;
  timestamp: string;
  prev_hash: string;
  merkle_root: string;
  nonce: number;
  hash: string;
  /**
   * Proof of stake only: who forged it, the reveal they showed, and their signature over the hash.
   */
  validator?: string;
  reveal?: string;
  signature?: string;
}

export interface Block extends BlockHeader {
  /** The payout first, then the transfers. */
  transactions: Transfer[];
}

/** One level of a Merkle proof: the sibling hash, and which side of the hash so far it sits on. */
export interface ProofStep {
  hash: string;
  side: "left" | "right";
}
