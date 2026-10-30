// Re-checking a block: its header hash, and that it meets the work it needed. Part of a domain
// service, verification.

import type { BlockHeader } from "@domain/blocks";
import { sha256Hex } from "@domain/encoding";

/** The timestamp as the node hashes it: always nine fractional digits. Its JSON trims them. */
function hashedTimestamp(iso: string): string {
  const match = iso.match(/^(.*T\d\d:\d\d:\d\d)(?:\.(\d+))?Z$/);
  if (!match) throw new Error(`Not a timestamp: ${iso}`);
  const [, whole, fraction = ""] = match;
  return `${whole}.${fraction.padEnd(9, "0")}Z`;
}

/**
 * `index|timestamp|prev_hash|merkle_root|nonce`, then the validator and reveal if present. Works on
 * a block or its header.
 */
export function headerHash(block: BlockHeader): Promise<string> {
  const extras = [block.validator, block.reveal]
    .filter(Boolean)
    .map((x) => `|${x}`)
    .join("");
  return sha256Hex(
    `${block.index}|${hashedTimestamp(block.timestamp)}|${block.prev_hash}|${block.merkle_root}|${block.nonce}${extras}`,
  );
}

/** Proof of work: the hash's first 128 bits are at most 2^128 / work. */
export const meetsWork = (hash: string, work: number): boolean =>
  BigInt(`0x${hash.slice(0, 32)}`) <= ((1n << 128n) - 1n) / BigInt(work);
