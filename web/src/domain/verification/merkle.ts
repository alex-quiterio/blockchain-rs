// Re-checking a block's Merkle tree: its root from its transactions, and the path a proof takes
// from one transaction up to it. Part of a domain service, verification: the node sends these,
// and this page works them out again rather than trust it.

import type { ProofStep } from "@domain/blocks";
import { sha256Hex } from "@domain/encoding";
import type { UnsignedTransfer } from "@domain/transfers";

export const EMPTY_ROOT = "0".repeat(64);

export const leaf = (tx: UnsignedTransfer): Promise<string> =>
  sha256Hex(`leaf|${JSON.stringify(tx)}`);
const node = (left: string, right: string): Promise<string> =>
  sha256Hex(`node|${left}|${right}`);

export async function root(
  transactions: readonly UnsignedTransfer[],
): Promise<string> {
  let level = await Promise.all(transactions.map(leaf));
  if (!level.length) return EMPTY_ROOT;
  while (level.length > 1) {
    const pairs: (string | Promise<string>)[] = [];
    for (let i = 0; i < level.length; i += 2) {
      // An odd hash out moves up as it is.
      pairs.push(
        i + 1 < level.length ? node(level[i]!, level[i + 1]!) : level[i]!,
      );
    }
    level = await Promise.all(pairs);
  }
  return level[0]!;
}

/** Follows a proof's steps from a leaf; returns every hash on the way up, the root last. */
export async function climb(
  fromLeaf: string,
  steps: readonly ProofStep[],
): Promise<string[]> {
  const path = [fromLeaf];
  for (const step of steps) {
    const here = path.at(-1)!;
    path.push(
      await (step.side === "left"
        ? node(step.hash, here)
        : node(here, step.hash)),
    );
  }
  return path;
}
