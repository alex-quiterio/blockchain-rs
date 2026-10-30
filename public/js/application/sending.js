// Sending from a wallet: a transfer to an address, or coins into or out of its stake. The page
// signs; only the signed transfer goes to the node.

import { signTransfer } from "../domain/wallets/keys.js";
import { commitment } from "../domain/wallets/reveals.js";
import { STAKE } from "../domain/transfers.js";
import { ports } from "./ports.js";
import { wallets } from "./wallets.js";

/** Signs and submits `amount` from the wallet `from` to `to`, an address, STAKE or UNSTAKE, with `fee`. */
export async function send({ from, to, amount, fee }) {
  const secret = wallets().secretOf(from.address);
  const { next_nonce, commitment_due } = await ports.node.balance(from.address);
  // A first stake commits to the reveal chain this wallet will forge with.
  const committed = to === STAKE && commitment_due != null ? await commitment(secret, commitment_due) : undefined;
  const tx = await signTransfer(secret, { from: from.address, to, amount, nonce: next_nonce, fee, commitment: committed });
  await ports.node.submitTransaction(tx);
  return tx;
}
