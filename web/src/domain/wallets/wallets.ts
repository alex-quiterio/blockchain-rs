// The page's wallets: an aggregate. Names and addresses are kept in the clear, so balances show
// even while locked; secret keys only in the vault, and in memory only while unlocked. Its
// invariant: nothing can read a secret key, so nothing can sign, while it's locked.
//
// It's kept through a repository it's given, a port it declares here; it doesn't know where.

import type { Keypair } from "./keys.ts";
import * as Vault from "./vault.ts";
import type { SealedVault, Secrets, Session } from "./vault.ts";

/** A wallet as listed: its name and address, never its key. */
export interface Wallet {
  name: string;
  address: string;
}

/** A wallet an older version saved unencrypted, waiting for a passphrase to move into the vault. */
export interface LegacyWallet extends Wallet {
  secret_key?: string;
}

/**
 * What's saved: the public list, the sealed vault, and any unencrypted wallets from older versions.
 */
export interface SavedWallets {
  accounts: Wallet[];
  vault: SealedVault | null;
  legacy: LegacyWallet[];
}

/** Where the wallets are kept: a repository, the port the aggregate is stored through. */
export interface WalletsRepository {
  load(): SavedWallets;
  save(saved: { accounts: Wallet[]; vault: SealedVault }): void;
  forgetLegacy(): void;
}

interface Backup {
  format: "blockchain-rs-backup";
  version: 1;
  accounts: Wallet[];
  vault: SealedVault;
}

export class Wallets {
  static IDLE_MS = 15 * 60 * 1000;

  #repository: WalletsRepository;
  list: Wallet[] = [];
  /** The encrypted secret keys, as saved. */
  vault: SealedVault | null = null;
  /** Unencrypted wallets from older versions, waiting for a passphrase. */
  legacy: Required<LegacyWallet>[] = [];
  /** Each wallet's secret key, only while unlocked. */
  secrets: Secrets | null = null;
  /** The key derived from the passphrase, while unlocked. */
  session: Session | null = null;
  lastActive = Date.now();

  constructor(repository: WalletsRepository) {
    this.#repository = repository;
  }

  /**
   * Reads what was saved. Unencrypted wallets from older versions are listed, to move into the
   * vault.
   */
  load(): this {
    const { accounts, vault, legacy } = this.#repository.load();
    this.list = accounts;
    this.vault = vault;
    this.legacy = legacy.filter((w): w is Required<LegacyWallet> =>
      Boolean(w.secret_key),
    );
    for (const w of this.legacy) {
      if (!this.byAddress(w.address))
        this.list.push({ name: w.name, address: w.address });
    }
    return this;
  }

  get locked(): boolean {
    return !this.secrets;
  }
  get needsPassphrase(): boolean {
    return !this.vault;
  }

  /**
   * First run: choose a passphrase. Any unencrypted wallets from older versions move into the
   * vault.
   */
  async setUp(passphrase: string): Promise<void> {
    const salt = crypto.getRandomValues(new Uint8Array(16));
    this.session = {
      salt,
      iterations: Vault.ITERATIONS,
      key: await Vault.deriveKey(passphrase, salt, Vault.ITERATIONS),
    };
    this.secrets = {};
    await this.#adoptLegacy();
  }

  async unlock(passphrase: string): Promise<void> {
    if (!this.vault) throw new Error("Choose a passphrase first");
    const { secrets, session } = await Vault.open(this.vault, passphrase);
    Object.assign(this, { secrets, session, lastActive: Date.now() });
    await this.#adoptLegacy();
  }

  lock(): void {
    this.secrets = null;
    this.session = null;
  }

  async #adoptLegacy(): Promise<void> {
    const secrets = this.#unlocked();
    for (const w of this.legacy) secrets[w.address] = w.secret_key;
    await this.#save();
    this.legacy = [];
    this.#repository.forgetLegacy();
  }

  async #save(): Promise<void> {
    if (!this.secrets || !this.session)
      throw new Error("Unlock your wallets first");
    this.vault = await Vault.seal(this.secrets, this.session);
    this.#repository.save({ accounts: this.list, vault: this.vault });
  }

  /**
   * The secret keys, touching the idle clock. Throws while locked, so nothing can sign without the
   * passphrase.
   */
  #unlocked(): Secrets {
    if (!this.secrets) throw new Error("Unlock your wallets first");
    this.lastActive = Date.now();
    return this.secrets;
  }

  async add({
    name,
    address,
    secret_key,
  }: Keypair & { name: string }): Promise<void> {
    const secrets = this.#unlocked();
    this.list.push({ name, address });
    secrets[address] = secret_key;
    await this.#save();
  }

  async remove(wallet: Wallet): Promise<void> {
    const secrets = this.#unlocked();
    this.list = this.list.filter((w) => w.address !== wallet.address);
    delete secrets[wallet.address];
    await this.#save();
  }

  /** A wallet's secret key. Throws while locked, so nothing can sign without the passphrase. */
  secretOf(address: string): string {
    const secret = this.#unlocked()[address];
    if (!secret) throw new Error("That wallet's key isn't in this browser");
    return secret;
  }

  byAddress(address: string): Wallet | undefined {
    return this.list.find((w) => w.address === address);
  }

  /** Whether it's been left unlocked and idle long enough to lock itself. */
  get idle(): boolean {
    return !this.locked && Date.now() - this.lastActive > Wallets.IDLE_MS;
  }

  /** The encrypted vault plus the public list. Useless without the passphrase. */
  backup(): string {
    const backup: Backup = {
      format: "blockchain-rs-backup",
      version: 1,
      accounts: this.list,
      vault: this.vault as SealedVault,
    };
    return JSON.stringify(backup, null, 2);
  }

  /** Merges a backup into this vault. Asks for the passphrase the backup was made with. */
  async restore(text: string, passphrase: string): Promise<number> {
    const ours = this.#unlocked();
    const backup = JSON.parse(text) as Backup;
    if (backup.format !== "blockchain-rs-backup")
      throw new Error("That file isn't a blockchain-rs backup");
    const { secrets } = await Vault.open(backup.vault, passphrase);
    let added = 0;
    for (const account of backup.accounts) {
      const secret = secrets[account.address];
      if (!secret || this.byAddress(account.address)) continue;
      this.list.push(account);
      ours[account.address] = secret;
      added += 1;
    }
    await this.#save();
    return added;
  }
}
