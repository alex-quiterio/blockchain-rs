// Adding a block: mining one under proof of work, or forging one under proof of stake.

import { sign } from "../domain/wallets/keys.js";
import { reveal } from "../domain/wallets/reveals.js";
import { ports } from "./ports.js";
import { wallets } from "./wallets.js";

/** Proof of work: the node searches for the block; `miner` gets the reward. */
export const mine = (miner) => ports.node.mine(miner);

/**
 * Proof of stake: the node proposes the block, this page signs its hash with `wallet`'s key, and
 * only the signature and the reveal go back. A reveal is due once validators are drawn by stake.
 */
export async function forge(wallet, validatorDrawn) {
  const secret = wallets().secretOf(wallet.address);
  const account = await ports.node.balance(wallet.address);
  const due = validatorDrawn && account.reveals_left;
  const next = due ? await reveal(secret, account.next_epoch - 1, account.reveals_left) : undefined;
  const block = await ports.node.propose(wallet.address, next);
  block.signature = await sign(secret, block.hash);
  await ports.node.submitBlock(block);
  return block;
}
