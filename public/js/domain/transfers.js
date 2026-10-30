// What a transfer is to this page: who it moves coins between, what kind it is, what it costs in
// fees, and the exact text its sender signs. Value-object rules, with no state of their own.

/** The names a sender or recipient takes when it isn't an account. */
export const COINBASE = "coinbase";
export const STAKE = "stake";
export const UNSTAKE = "unstake";

/** `coinbase`, `stake`, `unstake` or `transfer`. */
export const kindOf = (tx) => (tx.from === COINBASE ? "coinbase" : tx.to === STAKE ? "stake" : tx.to === UNSTAKE ? "unstake" : "transfer");

export const feesOf = (txs) => txs.reduce((sum, tx) => sum + (tx.fee || 0), 0);

/** What the sender signs: `from|to|amount|nonce`, then `|fee=N` and `|commit=C` if present. */
export function signingPayload(tx) {
  const extras = `${tx.fee ? `|fee=${tx.fee}` : ""}${tx.commitment ? `|commit=${tx.commitment}` : ""}`;
  return `${tx.from}|${tx.to}|${tx.amount}|${tx.nonce}${extras}`;
}
