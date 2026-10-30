// Re-checking signatures: a transfer's by its sender, and a forged block's by its validator. Part
// of a domain service, verification.

import type { BlockHeader } from "@domain/blocks";
import { fromHex } from "@domain/encoding";
import { signingPayload, type Transfer } from "@domain/transfers";

/** Whether `signature` is `address`'s ed25519 signature over `message`. */
export async function isSignedBy(
  address: string,
  message: string,
  signature: string,
): Promise<boolean> {
  try {
    const key = await crypto.subtle.importKey(
      "raw",
      fromHex(address),
      { name: "Ed25519" },
      false,
      ["verify"],
    );
    return await crypto.subtle.verify(
      { name: "Ed25519" },
      key,
      fromHex(signature),
      new TextEncoder().encode(message),
    );
  } catch {
    return false;
  }
}

/** Whether a transfer is signed by its sender, over exactly what it says. */
export const transferIsSigned = (tx: Transfer): Promise<boolean> =>
  isSignedBy(tx.from, signingPayload(tx), tx.signature);

/** Whether a forged block is signed by its validator, over its hash. */
export const blockIsSigned = (block: BlockHeader): Promise<boolean> =>
  block.validator && block.signature
    ? isSignedBy(block.validator, block.hash, block.signature)
    : Promise.resolve(false);
