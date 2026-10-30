// The page's wallets: an aggregate. Names and addresses are kept in the clear, so balances show
// even while locked; secret keys only in the vault, and in memory only while unlocked. Its
// invariant: nothing can read a secret key, so nothing can sign, while it's locked.
//
// It's kept through a repository it's given, with `load()`, `save({ accounts, vault })` and
// `forgetLegacy()`; the aggregate doesn't know where.

import * as Vault from "./vault.js";

export class Wallets {
  static IDLE_MS = 15 * 60 * 1000;

  #repository;

  constructor(repository) {
    this.#repository = repository;
    this.list = []; // [{ name, address }]: public
    this.vault = null; // the encrypted secret keys, as saved
    this.legacy = []; // unencrypted wallets from older versions, waiting for a passphrase
    this.secrets = null; // { address: secret_key }, only while unlocked
    this.session = null; // the key derived from the passphrase, while unlocked
    this.lastActive = Date.now();
  }

  /** Reads what was saved. Unencrypted wallets from older versions are listed, to move into the vault. */
  load() {
    const { accounts, vault, legacy } = this.#repository.load();
    this.list = accounts;
    this.vault = vault;
    this.legacy = legacy.filter((w) => w.secret_key);
    for (const w of this.legacy) {
      if (!this.byAddress(w.address)) this.list.push({ name: w.name, address: w.address });
    }
    return this;
  }

  get locked() { return !this.secrets; }
  get needsPassphrase() { return !this.vault; }

  /** First run: choose a passphrase. Any unencrypted wallets from older versions move into the vault. */
  async setUp(passphrase) {
    const salt = crypto.getRandomValues(new Uint8Array(16));
    this.session = { salt, iterations: Vault.ITERATIONS, key: await Vault.deriveKey(passphrase, salt, Vault.ITERATIONS) };
    this.secrets = {};
    await this.#adoptLegacy();
  }

  async unlock(passphrase) {
    const { secrets, session } = await Vault.open(this.vault, passphrase);
    Object.assign(this, { secrets, session, lastActive: Date.now() });
    await this.#adoptLegacy();
  }

  lock() {
    this.secrets = null;
    this.session = null;
  }

  async #adoptLegacy() {
    for (const w of this.legacy) this.secrets[w.address] = w.secret_key;
    await this.#save();
    this.legacy = [];
    this.#repository.forgetLegacy();
  }

  async #save() {
    this.vault = await Vault.seal(this.secrets, this.session);
    this.#repository.save({ accounts: this.list, vault: this.vault });
  }

  async add({ name, address, secret_key }) {
    this.secretOf(null);
    this.list.push({ name, address });
    this.secrets[address] = secret_key;
    await this.#save();
  }

  async remove(wallet) {
    this.secretOf(null);
    this.list = this.list.filter((w) => w.address !== wallet.address);
    delete this.secrets[wallet.address];
    await this.#save();
  }

  /** A wallet's secret key. Throws while locked, so nothing can sign without the passphrase. */
  secretOf(address) {
    if (this.locked) throw new Error("Unlock your wallets first");
    this.lastActive = Date.now();
    return address && this.secrets[address];
  }

  byAddress(address) { return this.list.find((w) => w.address === address); }

  /** Whether it's been left unlocked and idle long enough to lock itself. */
  get idle() { return !this.locked && Date.now() - this.lastActive > Wallets.IDLE_MS; }

  /** The encrypted vault plus the public list. Useless without the passphrase. */
  backup() {
    return JSON.stringify({ format: "blockchain-rs-backup", version: 1, accounts: this.list, vault: this.vault }, null, 2);
  }

  /** Merges a backup into this vault. Asks for the passphrase the backup was made with. */
  async restore(text, passphrase) {
    this.secretOf(null);
    const backup = JSON.parse(text);
    if (backup.format !== "blockchain-rs-backup") throw new Error("That file isn't a blockchain-rs backup");
    const { secrets } = await Vault.open(backup.vault, passphrase);
    let added = 0;
    for (const account of backup.accounts) {
      if (!secrets[account.address] || this.byAddress(account.address)) continue;
      this.list.push(account);
      this.secrets[account.address] = secrets[account.address];
      added += 1;
    }
    await this.#save();
    return added;
  }
}
