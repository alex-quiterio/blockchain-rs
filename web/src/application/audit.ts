// Auditing what the node sends: every check the pages show, decided here, so the pages only say
// what each one means. A check that takes hashing or a signature comes back as a promise, so a
// page can show it as it finishes; the rest come back at once. Each verdict carries the facts a
// page shows beside it.

import type { Block, ProofStep } from "@domain/blocks";
import { feesOf, kindOf, type Transfer } from "@domain/transfers";
import { headerHash, meetsWork } from "@domain/verification/blocks";
import { climb, leaf, root } from "@domain/verification/merkle";
import {
  blockIsSigned,
  transferIsSigned,
} from "@domain/verification/signatures";
import { COINBASE, STAKE, UNSTAKE } from "@domain/transfers";
import type { FoundTransaction, Lookup } from "./lookup.ts";
import type { Account, Located } from "./ports.ts";

/** How a block was sealed, and whether the seal holds: a validator's signature, or enough work. */
export type Seal =
  | { by: "signature"; validator: string; passed: Promise<boolean> }
  | { by: "work"; work: number | undefined; passed: boolean };

export interface BlockAudit {
  block: Block;
  /** The hash recomputed from the header matches the block's. */
  header: Promise<boolean>;
  /** The block names the one before it by its hash. `null` for genesis, which has none. */
  link: { passed: boolean; prev: Block } | null;
  seal: Seal;
  /** The Merkle root recomputed from the transactions matches the header's. */
  contents: Promise<boolean>;
  /** How many transfers are signed by their senders. `null` when there are none. */
  signatures: Promise<{ verified: number; of: number }> | null;
  /** The payout is the reward plus the block's fees. `null` for genesis. */
  payout: {
    passed: boolean;
    reward: number;
    fees: number;
    amount: number;
    to: string;
  } | null;
}

/** A block by height or hash, looked up through `lookup`, with every check of it under way. */
export async function auditBlock(
  lookup: Lookup,
  id: string,
): Promise<BlockAudit> {
  const { block, prev, work } = await lookup.block(id);
  const [payout, ...transfers] = block.transactions;
  const fees = feesOf(transfers);
  return {
    block,
    header: headerHash(block).then((h) => h === block.hash),
    link: prev ? { passed: prev.hash === block.prev_hash, prev } : null,
    seal: block.validator
      ? {
          by: "signature",
          validator: block.validator,
          passed: blockIsSigned(block),
        }
      : {
          by: "work",
          work,
          passed: work != null && meetsWork(block.hash, work),
        },
    contents: contentsMatch(block),
    signatures: transfers.length
      ? Promise.all(transfers.map(transferIsSigned)).then((results) => ({
          verified: results.filter(Boolean).length,
          of: results.length,
        }))
      : null,
    payout: payout
      ? {
          passed: payout.amount === lookup.rules().reward + fees,
          reward: lookup.rules().reward,
          fees,
          amount: payout.amount,
          to: payout.to,
        }
      : null,
  };
}

/** Whether a block's Merkle root is the one its transactions hash up to. */
export const contentsMatch = (block: Block): Promise<boolean> =>
  root(block.transactions).then((r) => r === block.merkle_root);

/** Whether a transfer is signed by its sender; `null` for a block's payout, which isn't signed. */
export const signatureOf = (tx: Transfer): Promise<boolean | null> =>
  kindOf(tx) === "coinbase" ? Promise.resolve(null) : transferIsSigned(tx);

/** The Merkle proof that a confirmed transfer is in its block, followed up from its leaf. */
export interface InclusionAudit {
  /** The leaf hashed here, and every hash on the way up, the root last. */
  path: string[];
  steps: ProofStep[];
  /** The leaf hashed here is the node's. */
  leafMatches: boolean;
  /** The path ends at the header's Merkle root. */
  reachesRoot: boolean;
  /** The header hashes to the block the transfer is placed in. */
  header: Promise<boolean>;
  /** The header's height and Merkle root, which a page shows. */
  height: number;
  root: string;
}

export interface TransactionAudit extends FoundTransaction {
  /** The transfer is signed by its sender, over exactly what it says. */
  signature: Promise<boolean>;
  /** Once confirmed, the proof that it's in its block. */
  inclusion: Promise<InclusionAudit> | null;
}

/** A transaction by signature, with its signature and, once confirmed, its inclusion checked. */
export async function auditTransaction(
  lookup: Lookup,
  signature: string,
): Promise<TransactionAudit> {
  const found = await lookup.transaction(signature);
  const { transaction: tx, placed, proof } = found;
  const inclusion =
    placed && proof
      ? leaf(tx).then(async (ownLeaf): Promise<InclusionAudit> => {
          const path = await climb(ownLeaf, proof.proof);
          const header = proof.header;
          return {
            path,
            steps: proof.proof,
            leafMatches: ownLeaf === proof.leaf,
            reachesRoot: path.at(-1) === header.merkle_root,
            header: headerHash(header).then(
              (h) => h === header.hash && h === placed.hash,
            ),
            height: header.index,
            root: header.merkle_root,
          };
        })
      : null;
  return { ...found, signature: transferIsSigned(tx), inclusion };
}

/** What an address's confirmed history adds up to. */
export interface Totals {
  rewards: number;
  received: number;
  sent: number;
  fees: number;
  staked: number;
}

export interface AddressAudit {
  account: Account;
  history: Located[];
  totals: Totals;
  /** In − out − stake is the balance, and the stake is what's staked, from the history alone. */
  addsUp: boolean;
}

/** An address's account and history, and whether the history adds up to the account. */
export async function auditAddress(
  lookup: Lookup,
  address: string,
): Promise<AddressAudit> {
  const { account, history } = await lookup.address(address);
  const totals = totalsOf(address, history);
  const { rewards, received, sent, fees, staked } = totals;
  return {
    account,
    history,
    totals,
    addsUp:
      rewards + received - sent - fees - staked === account.balance &&
      staked === account.staked,
  };
}

/** What an address's confirmed transactions add up to. */
export function totalsOf(address: string, history: readonly Located[]): Totals {
  const confirmed = history.filter((h) => h.block).map((h) => h.transaction);
  const sum = (pick: (tx: Transfer) => number) =>
    confirmed.reduce((total, tx) => total + pick(tx), 0);
  const ours = (tx: Transfer) => tx.from === address;
  return {
    rewards: sum((tx) =>
      tx.to === address && tx.from === COINBASE ? tx.amount : 0,
    ),
    received: sum((tx) =>
      tx.to === address && tx.from !== COINBASE ? tx.amount : 0,
    ),
    sent: sum((tx) => (ours(tx) && kindOf(tx) === "transfer" ? tx.amount : 0)),
    fees: sum((tx) => (ours(tx) ? tx.fee || 0 : 0)),
    staked:
      sum((tx) => (ours(tx) && tx.to === STAKE ? tx.amount : 0)) -
      sum((tx) => (ours(tx) && tx.to === UNSTAKE ? tx.amount : 0)),
  };
}
