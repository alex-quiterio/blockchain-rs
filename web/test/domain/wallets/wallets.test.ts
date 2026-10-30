import assert from "node:assert/strict";
import { test } from "node:test";
import { generateKeypair } from "@domain/wallets/keys";
import {
  Wallets,
  type SavedWallets,
  type WalletsRepository,
} from "@domain/wallets/wallets";

/** A repository in memory, recording what was saved. */
function memory(saved: Partial<SavedWallets> = {}) {
  const store: SavedWallets = {
    accounts: [],
    vault: null,
    legacy: [],
    ...saved,
  };
  let forgotLegacy = false;
  const repository: WalletsRepository = {
    load: () => structuredClone(store),
    save: ({ accounts, vault }) =>
      Object.assign(store, structuredClone({ accounts, vault })),
    forgetLegacy: () => {
      forgotLegacy = true;
    },
  };
  return { repository, store, forgotLegacy: () => forgotLegacy };
}

test("nothing can read a key while locked, and a wallet is kept through its repository", async () => {
  const { repository, store } = memory();
  const wallets = new Wallets(repository).load();
  assert.ok(wallets.needsPassphrase && wallets.locked);
  await wallets.setUp("correct horse");
  const alice = { ...(await generateKeypair()), name: "Alice" };
  await wallets.add(alice);
  assert.equal(wallets.secretOf(alice.address), alice.secret_key);
  assert.deepEqual(store.accounts, [{ name: "Alice", address: alice.address }]);
  assert.ok(store.vault, "the vault is saved");

  wallets.lock();
  assert.throws(
    () => wallets.secretOf(alice.address),
    /Unlock your wallets first/,
  );
  await assert.rejects(
    wallets.add({ ...(await generateKeypair()), name: "Bob" }),
    /Unlock/,
  );

  // Opening again, from what was saved: still listed while locked, unlocked by the passphrase.
  const again = new Wallets(repository).load();
  assert.deepEqual(again.list, [{ name: "Alice", address: alice.address }]);
  await assert.rejects(again.unlock("wrong horse"), /Wrong passphrase/);
  await again.unlock("correct horse");
  assert.equal(again.secretOf(alice.address), alice.secret_key);
});

test("a key that isn't in this browser is said to be missing", async () => {
  const wallets = new Wallets(memory().repository).load();
  await wallets.setUp("correct horse");
  assert.throws(
    () => wallets.secretOf("cc".repeat(32)),
    /isn't in this browser/,
  );
});

test("unencrypted wallets from older versions move into the vault", async () => {
  const old = await generateKeypair();
  const { repository, store, forgotLegacy } = memory({
    legacy: [{ name: "Old", ...old }],
  });
  const wallets = new Wallets(repository).load();
  assert.deepEqual(
    wallets.list,
    [{ name: "Old", address: old.address }],
    "listed before a passphrase",
  );
  await wallets.setUp("correct horse");
  assert.equal(wallets.secretOf(old.address), old.secret_key);
  assert.ok(forgotLegacy(), "the plain copy is deleted");
  assert.deepEqual(store.accounts, [{ name: "Old", address: old.address }]);
});

test("a backup restores into another vault, adding only the wallets it lacks", async () => {
  const mine = new Wallets(memory().repository).load();
  await mine.setUp("correct horse");
  const alice = { ...(await generateKeypair()), name: "Alice" };
  await mine.add(alice);

  const theirs = new Wallets(memory().repository).load();
  await theirs.setUp("another passphrase");
  await assert.rejects(
    theirs.restore(mine.backup(), "another passphrase"),
    /Wrong passphrase/,
  );
  assert.equal(await theirs.restore(mine.backup(), "correct horse"), 1);
  assert.equal(theirs.secretOf(alice.address), alice.secret_key);
  assert.equal(
    await theirs.restore(mine.backup(), "correct horse"),
    0,
    "already there",
  );
  await assert.rejects(
    theirs.restore('{"format":"other"}', "x"),
    /isn't a blockchain-rs backup/,
  );
});

test("left unlocked and untouched long enough, the wallets are idle", async () => {
  const wallets = new Wallets(memory().repository).load();
  assert.equal(wallets.idle, false, "locked wallets are never idle");
  await wallets.setUp("correct horse");
  assert.equal(wallets.idle, false);
  wallets.lastActive = Date.now() - Wallets.IDLE_MS - 1;
  assert.equal(wallets.idle, true);
});
