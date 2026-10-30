// The page's wallets, and making a new one.

import { generateKeypair } from "../domain/wallets/keys.js";
import { Wallets } from "../domain/wallets/wallets.js";
import { ports } from "./ports.js";

let loaded = null;

/** The wallets, loaded from their repository the first time they're asked for. */
export const wallets = () => (loaded ??= new Wallets(ports.walletsRepository).load());

/** A new wallet with a fresh keypair, named `name` or numbered after the others. */
export async function createWallet(name) {
  const { address, secret_key } = await generateKeypair();
  const named = name || `Wallet ${wallets().list.length + 1}`;
  await wallets().add({ name: named, address, secret_key });
  return named;
}
