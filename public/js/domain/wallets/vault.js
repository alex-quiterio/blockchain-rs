// Secret keys encrypted with a passphrase before they're saved. A domain service.

import { fromHex, toHex } from "../encoding.js";

// OWASP's 2023 recommendation for PBKDF2-SHA256: slow enough to make guessing expensive.
export const ITERATIONS = 600_000;

export async function deriveKey(passphrase, salt, iterations) {
  const material = await crypto.subtle.importKey("raw", new TextEncoder().encode(passphrase), "PBKDF2", false, ["deriveKey"]);
  return crypto.subtle.deriveKey(
    { name: "PBKDF2", hash: "SHA-256", salt, iterations },
    material, { name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"]);
}

/** Encrypts `secrets` under a session's key, with a fresh IV every time. */
export async function seal(secrets, session) {
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const plain = new TextEncoder().encode(JSON.stringify(secrets));
  const data = new Uint8Array(await crypto.subtle.encrypt({ name: "AES-GCM", iv }, session.key, plain));
  return { version: 1, kdf: "PBKDF2-SHA256", iterations: session.iterations, salt: toHex(session.salt), iv: toHex(iv), data: toHex(data) };
}

/** Decrypts a sealed vault. AES-GCM refuses a wrong key outright, so a bad passphrase can't half-work. */
export async function open(vault, passphrase) {
  const session = { salt: fromHex(vault.salt), iterations: vault.iterations };
  session.key = await deriveKey(passphrase, session.salt, session.iterations);
  try {
    const plain = await crypto.subtle.decrypt({ name: "AES-GCM", iv: fromHex(vault.iv) }, session.key, fromHex(vault.data));
    return { secrets: JSON.parse(new TextDecoder().decode(plain)), session };
  } catch {
    throw new Error("Wrong passphrase");
  }
}
