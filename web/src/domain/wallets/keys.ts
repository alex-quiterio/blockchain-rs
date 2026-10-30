// Making keys and signing with them. A domain service: the keys are made and used in this page
// and never leave it.

import { fromHex, toHex } from "@domain/encoding";
import {
  signingPayload,
  type Transfer,
  type UnsignedTransfer,
} from "@domain/transfers";

const ALGORITHM = { name: "Ed25519" };
// A PKCS#8 wrapper around a 32-byte ed25519 seed, the form WebCrypto imports private keys in.
const PKCS8_PREFIX = "302e020100300506032b657004220420";

/** A keypair: the address is the public key, both hex. */
export interface Keypair {
  address: string;
  secret_key: string;
}

export async function generateKeypair(): Promise<Keypair> {
  const pair = (await crypto.subtle.generateKey(ALGORITHM, true, [
    "sign",
    "verify",
  ])) as CryptoKeyPair;
  const publicKey = new Uint8Array(
    await crypto.subtle.exportKey("raw", pair.publicKey),
  );
  const pkcs8 = new Uint8Array(
    await crypto.subtle.exportKey("pkcs8", pair.privateKey),
  );
  return { address: toHex(publicKey), secret_key: toHex(pkcs8.slice(-32)) };
}

export async function sign(
  secretKey: string,
  message: string,
): Promise<string> {
  const key = await crypto.subtle.importKey(
    "pkcs8",
    fromHex(PKCS8_PREFIX + secretKey),
    ALGORITHM,
    false,
    ["sign"],
  );
  return toHex(
    new Uint8Array(
      await crypto.subtle.sign(
        ALGORITHM,
        key,
        new TextEncoder().encode(message),
      ),
    ),
  );
}

/**
 * A transfer signed with `secretKey`. A fee or commitment is left out when there's none, as the
 * node expects.
 */
export async function signTransfer(
  secretKey: string,
  { from, to, amount, nonce, fee, commitment }: UnsignedTransfer,
): Promise<Transfer> {
  const tx: UnsignedTransfer = {
    from,
    to,
    amount,
    nonce,
    ...(fee ? { fee } : {}),
    ...(commitment ? { commitment } : {}),
  };
  return { ...tx, signature: await sign(secretKey, signingPayload(tx)) };
}
