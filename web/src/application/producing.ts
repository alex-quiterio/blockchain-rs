// Adding a block: mining one under proof of work, or forging one under proof of stake.

import type { Block } from "@domain/blocks";
import { sign } from "@domain/wallets/keys";
import { reveal } from "@domain/wallets/reveals";
import type { Wallet, Wallets } from "@domain/wallets/wallets";
import type { NodePort } from "./ports.ts";

/** Proof of work: the node searches for the block; `miner` gets the reward. */
export const mine = (node: NodePort, miner: string): Promise<Block> =>
  node.mine(miner);

/**
 * Proof of stake: the node proposes the block, this page signs its hash with `wallet`'s key, and
 * only the signature and the reveal go back. A reveal is due once validators are drawn by stake.
 */
export async function forge(
  node: NodePort,
  wallets: Wallets,
  wallet: Wallet,
  validatorDrawn: boolean,
): Promise<Block> {
  const secret = wallets.secretOf(wallet.address);
  const account = await node.balance(wallet.address);
  const due = validatorDrawn && account.reveals_left;
  const next = due
    ? await reveal(secret, account.next_epoch - 1, due)
    : undefined;
  const block = await node.propose(wallet.address, next);
  block.signature = await sign(secret, block.hash);
  await node.submitBlock(block);
  return block;
}
