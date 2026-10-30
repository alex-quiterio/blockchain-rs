// The wallets' repository, kept in the browser's localStorage: an adapter for the domain's
// WalletsRepository. In private mode storage may refuse; the wallets then last as long as the tab.

import type { WalletsRepository } from "@domain/wallets/wallets";

const ACCOUNTS = "blockchain-rs:accounts";
const VAULT = "blockchain-rs:vault";
const LEGACY = "blockchain-rs:wallets"; // older versions kept keys here unencrypted

function read<T>(key: string, fallback: T): T {
  try {
    return (
      (JSON.parse(localStorage.getItem(key) ?? "null") as T | null) ?? fallback
    );
  } catch {
    return fallback;
  }
}

export const WalletsLocalStorageRepository: WalletsRepository = {
  load() {
    return {
      accounts: read(ACCOUNTS, []),
      vault: read(VAULT, null),
      legacy: read(LEGACY, []),
    };
  },

  save({ accounts, vault }) {
    try {
      localStorage.setItem(VAULT, JSON.stringify(vault));
      localStorage.setItem(ACCOUNTS, JSON.stringify(accounts));
    } catch {
      /* private mode */
    }
  },

  forgetLegacy() {
    try {
      localStorage.removeItem(LEGACY);
    } catch {
      /* private mode */
    }
  },
};
