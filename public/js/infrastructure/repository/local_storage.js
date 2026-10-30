// The wallets' repository, kept in the browser's localStorage. An adapter: the Wallets aggregate
// only knows it can `load`, `save` and `forgetLegacy`. In private mode storage may refuse; the
// wallets then last as long as the tab.

const ACCOUNTS = "blockchain-rs:accounts";
const VAULT = "blockchain-rs:vault";
const LEGACY = "blockchain-rs:wallets"; // older versions kept keys here unencrypted

function read(key, fallback) {
  try { return JSON.parse(localStorage.getItem(key)) ?? fallback; } catch { return fallback; }
}

export const WalletsLocalStorageRepository = {
  load() {
    return { accounts: read(ACCOUNTS, []), vault: read(VAULT, null), legacy: read(LEGACY, []) };
  },

  save({ accounts, vault }) {
    try {
      localStorage.setItem(VAULT, JSON.stringify(vault));
      localStorage.setItem(ACCOUNTS, JSON.stringify(accounts));
    } catch { /* private mode */ }
  },

  forgetLegacy() {
    try { localStorage.removeItem(LEGACY); } catch { /* private mode */ }
  },
};
