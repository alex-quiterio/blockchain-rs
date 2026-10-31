// Sending from a wallet: a transfer to an address, or coins into or out of its stake. The page
// signs; only the signed transfer goes to the node.

import { STAKE, type Transfer } from "@domain/transfers";
import { signTransfer } from "@domain/wallets/keys";
import { commitment } from "@domain/wallets/reveals";
import type { Wallet, Wallets } from "@domain/wallets/wallets";
import type { NodePort } from "./ports.ts";

export interface Sending {
  from: Wallet;
  /** An address, STAKE or UNSTAKE. */
  to: string;
  amount: number;
  fee: number;
}

/** Signs `amount` from `from` to `to`, with `fee`, with a key from `wallets`, and submits it. */
export async function send(
  node: NodePort,
  wallets: Wallets,
  { from, to, amount, fee }: Sending,
): Promise<Transfer> {
  const secret = wallets.secretOf(from.address);
  const [{ next_nonce, commitment_due }, { chain }] = await Promise.all([
    node.balance(from.address),
    node.status(),
  ]);
  // A first stake commits to the reveal chain this wallet will forge with.
  const committed =
    to === STAKE && commitment_due != null
      ? await commitment(secret, commitment_due)
      : undefined;
  const tx = await signTransfer(secret, {
    from: from.address,
    to,
    amount,
    nonce: next_nonce,
    fee,
    commitment: committed,
    chain,
  });
  await node.submitTransaction(tx);
  return tx;
}
