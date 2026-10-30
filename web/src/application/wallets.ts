// Making a new wallet among the page's wallets.

import { generateKeypair } from "@domain/wallets/keys";
import type { Wallets } from "@domain/wallets/wallets";

/** A new wallet with a fresh keypair, named `name` or numbered after the others. */
export async function createWallet(
  wallets: Wallets,
  name: string,
): Promise<string> {
  const { address, secret_key } = await generateKeypair();
  const named = name || `Wallet ${wallets.list.length + 1}`;
  await wallets.add({ name: named, address, secret_key });
  return named;
}
