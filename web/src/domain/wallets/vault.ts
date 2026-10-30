// Secret keys encrypted with a passphrase before they're saved. A domain service.

import { fromHex, toHex } from "@domain/encoding";

// OWASP's 2023 recommendation for PBKDF2-SHA256: slow enough to make guessing expensive.
export const ITERATIONS = 600_000;

/** Each wallet's secret key, by address. */
export type Secrets = Record<string, string>;

/** The key derived from a passphrase, while unlocked. */
export interface Session {
  salt: Uint8Array<ArrayBuffer>;
  iterations: number;
  key: CryptoKey;
}

/**
 * Secret keys as saved: encrypted, with what's needed to derive the key again from the passphrase.
 */
export interface SealedVault {
  version: 1;
  kdf: "PBKDF2-SHA256";
  iterations: number;
  salt: string;
  iv: string;
  data: string;
}

export async function deriveKey(
  passphrase: string,
  salt: Uint8Array<ArrayBuffer>,
  iterations: number,
): Promise<CryptoKey> {
  const material = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(passphrase),
    "PBKDF2",
    false,
    ["deriveKey"],
  );
  return crypto.subtle.deriveKey(
    { name: "PBKDF2", hash: "SHA-256", salt, iterations },
    material,
    { name: "AES-GCM", length: 256 },
    false,
    ["encrypt", "decrypt"],
  );
}

/** Encrypts `secrets` under a session's key, with a fresh IV every time. */
export async function seal(
  secrets: Secrets,
  session: Session,
): Promise<SealedVault> {
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const plain = new TextEncoder().encode(JSON.stringify(secrets));
  const data = new Uint8Array(
    await crypto.subtle.encrypt({ name: "AES-GCM", iv }, session.key, plain),
  );
  return {
    version: 1,
    kdf: "PBKDF2-SHA256",
    iterations: session.iterations,
    salt: toHex(session.salt),
    iv: toHex(iv),
    data: toHex(data),
  };
}

/**
 * Decrypts a sealed vault. AES-GCM refuses a wrong key outright, so a bad passphrase can't half-
 * work.
 */
export async function open(
  vault: SealedVault,
  passphrase: string,
): Promise<{ secrets: Secrets; session: Session }> {
  const salt = fromHex(vault.salt);
  const session: Session = {
    salt,
    iterations: vault.iterations,
    key: await deriveKey(passphrase, salt, vault.iterations),
  };
  try {
    const plain = await crypto.subtle.decrypt(
      { name: "AES-GCM", iv: fromHex(vault.iv) },
      session.key,
      fromHex(vault.data),
    );
    return {
      secrets: JSON.parse(new TextDecoder().decode(plain)) as Secrets,
      session,
    };
  } catch {
    throw new Error("Wrong passphrase");
  }
}
