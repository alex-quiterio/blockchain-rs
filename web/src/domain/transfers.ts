// What a transfer is to this page: who it moves coins between, what kind it is, what it costs in
// fees, and the exact text its sender signs. Value-object rules, with no state of their own.

/** The names a sender or recipient takes when it isn't an account. */
export const COINBASE = "coinbase";
export const STAKE = "stake";
export const UNSTAKE = "unstake";

/**
 * A transfer as its sender signs it. A fee or commitment is left out when there's none, and the
 * chain only by transfers made before they named one.
 */
export interface UnsignedTransfer {
  from: string;
  to: string;
  amount: number;
  nonce: number;
  fee?: number;
  commitment?: string;
  /** The chain it's for, its genesis hash, so it can't be replayed on another. */
  chain?: string;
}

/** A transfer as the node keeps it. A block's payout has an empty signature. */
export interface Transfer extends UnsignedTransfer {
  signature: string;
}

export type TransferKind = "coinbase" | "stake" | "unstake" | "transfer";

export const kindOf = (tx: UnsignedTransfer): TransferKind =>
  tx.from === COINBASE
    ? "coinbase"
    : tx.to === STAKE
      ? "stake"
      : tx.to === UNSTAKE
        ? "unstake"
        : "transfer";

export const feesOf = (txs: readonly UnsignedTransfer[]): number =>
  txs.reduce((sum, tx) => sum + (tx.fee || 0), 0);

/**
 * What the sender signs: `from|to|amount|nonce`, then `|fee=N`, `|commit=C` and `|chain=G` if
 * present.
 */
export function signingPayload(tx: UnsignedTransfer): string {
  const extras = `${tx.fee ? `|fee=${tx.fee}` : ""}${tx.commitment ? `|commit=${tx.commitment}` : ""}${tx.chain ? `|chain=${tx.chain}` : ""}`;
  return `${tx.from}|${tx.to}|${tx.amount}|${tx.nonce}${extras}`;
}
