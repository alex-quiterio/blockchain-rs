// The wallets panel: the vault's lock, the wallets and what each holds, making one, and backing
// them up or restoring them.

import { isStake, state } from "../../../../application/node_view.js";
import { createWallet, wallets } from "../../../../application/wallets.js";
import { $, el, fmt, short, toast } from "../../../dom.js";
import { busy, fail, refreshPage } from "../../actions.js";
import { avatar } from "../../components.js";
import { updateSendSummary } from "../sending/panel.js";

const lockIcon = (open) => el("span", { textContent: open ? "🔓" : "🔒", ariaHidden: "true" });

function passphraseInput(placeholder, autocomplete) {
  return el("input", { type: "password", placeholder, autocomplete, required: true, minLength: 8 });
}

/** Fills a select with the wallets, then `extras` as `[value, label]`, keeping what was chosen. */
export function fillSelect(select, extras = []) {
  const current = select.value;
  select.replaceChildren(
    ...wallets().list.map((w) => el("option", { value: w.address, textContent: w.name })),
    ...extras.map(([value, label]) => el("option", { value, textContent: label })),
  );
  if ([...select.options].some((o) => o.value === current)) select.value = current;
}

export function renderWallets() {
  renderVault();
  const list = wallets().list;
  $("wallet-count").textContent = list.length || "";
  $("wallets").replaceChildren(
    ...(list.length
      ? list.map(walletView)
      : [el("div", { className: "empty", textContent: `No wallets yet. Make one, then ${isStake() ? "forge" : "mine"} to fund it.` })]),
  );
  fillSelect($("send-from"));
  fillSelect($("send-to"), [
    isStake() && ["__stake", "Stake: lock coins to validate"],
    isStake() && ["__unstake", "Unstake: unlock staked coins"],
    ["__other", "Other address…"],
  ].filter(Boolean));
  updateSendSummary();
}

function walletView(wallet) {
  const account = state.accounts[wallet.address] || { balance: 0, staked: 0, next_nonce: 0 };
  const copy = el("button", { className: "icon", type: "button", textContent: "⧉", title: "Copy address" });
  copy.onclick = () => navigator.clipboard.writeText(wallet.address).then(() => toast("Address copied"));
  const forget = el("button", { className: "icon forget", type: "button", textContent: "✕", title: "Forget this wallet" });
  forget.onclick = () => {
    if (wallets().locked) return toast("Unlock your wallets first", { error: true });
    if (!confirm(`Forget ${wallet.name}? Its secret key will be gone from this browser.`)) return;
    wallets().remove(wallet).then(refreshPage, fail);
  };
  return el("div", { className: "wallet" },
    avatar(wallet.address),
    el("div", { className: "who" },
      el("div", { className: "name", textContent: wallet.name }),
      el("div", { className: "addr", title: wallet.address }, short(wallet.address), copy)),
    el("div", { className: "money" },
      el("div", { className: `balance${account.balance ? "" : " zero"}`, textContent: fmt(account.balance) }),
      account.staked
        ? el("div", { className: "staked", textContent: `${fmt(account.staked)} staked` })
        : el("div", { className: "nonce", textContent: `nonce ${account.next_nonce}` })),
    forget);
}

function renderVault() {
  const w = wallets();
  $("wallet-form").hidden = w.locked;
  if (w.needsPassphrase) return renderSetUp();
  if (w.locked) return renderUnlock();

  const lock = el("button", { className: "small", type: "button", textContent: "Lock" });
  lock.onclick = () => { w.lock(); refreshPage(); toast("Locked. Your keys are wiped from memory."); };
  const backup = el("button", { className: "small", type: "button", textContent: "Back up" });
  backup.onclick = downloadBackup;
  const restore = el("button", { className: "small", type: "button", textContent: "Restore" });
  restore.onclick = restoreBackup;
  $("vault").replaceChildren(
    el("div", { className: "unlocked" },
      lockIcon(true), el("span", { className: "title", textContent: "Unlocked" }),
      el("span", { className: "actions" }, lock, backup, restore)),
    el("div", { className: "unlocked-note", textContent: "Keys are in this tab's memory until you lock, close the tab or stay idle for 15 minutes." }));
}

function renderSetUp() {
  const first = passphraseInput("Passphrase, 8+ characters", "new-password");
  const again = passphraseInput("Type it again", "new-password");
  const button = el("button", { textContent: "Set passphrase" });
  const form = el("form", {}, first, again, button);
  form.onsubmit = (e) => {
    e.preventDefault();
    if (first.value !== again.value) return toast("The passphrases don't match", { error: true });
    busy(button, async () => {
      button.textContent = "Encrypting…";
      await wallets().setUp(first.value);
      toast("Your keys are now encrypted with your passphrase");
    });
  };
  const n = wallets().legacy.length;
  $("vault").replaceChildren(el("div", { className: "lockcard locked" },
    el("div", { className: "title" }, lockIcon(false), n ? `Protect your ${n} wallet${n === 1 ? "" : "s"}` : "Choose a passphrase"),
    el("p", { textContent: n
      ? "They're saved unencrypted from an older version. A passphrase encrypts them; the plain copy is then deleted."
      : "It encrypts your wallets' secret keys before they're saved in this browser. There's no way to reset it, so pick one you'll remember." }),
    form));
}

function renderUnlock() {
  const input = passphraseInput("Passphrase", "current-password");
  input.id = "unlock-passphrase";
  const button = el("button", { textContent: "Unlock" });
  const form = el("form", {}, el("div", { className: "row" }, el("span", { className: "grow" }, input), button));
  form.onsubmit = (e) => {
    e.preventDefault();
    busy(button, async () => {
      button.textContent = "Unlocking…";
      await wallets().unlock(input.value);
      toast("Unlocked");
    });
  };
  $("vault").replaceChildren(el("div", { className: "lockcard locked" },
    el("div", { className: "title" }, lockIcon(false), "Wallets locked"),
    el("p", { textContent: "Balances and mining work while locked. Unlock to send, forge or make wallets." }),
    form));
}

function downloadBackup() {
  const link = el("a", {
    href: URL.createObjectURL(new Blob([wallets().backup()], { type: "application/json" })),
    download: `blockchain-rs-wallets-${new Date().toISOString().slice(0, 10)}.json`,
  });
  link.click();
  URL.revokeObjectURL(link.href);
  toast("Saved an encrypted backup. You'll need the same passphrase to restore it.");
}

function restoreBackup() {
  const picker = el("input", { type: "file", accept: "application/json,.json" });
  picker.onchange = async () => {
    try {
      const text = await picker.files[0].text();
      const passphrase = prompt("Passphrase this backup was made with:");
      if (!passphrase) return;
      const added = await wallets().restore(text, passphrase);
      toast(added ? `Restored ${added} wallet${added === 1 ? "" : "s"}` : "Those wallets are already here");
      refreshPage();
    } catch (err) {
      fail(err);
    }
  };
  picker.click();
}

/** Wires up making a wallet, and locking the wallets after a stretch without activity. */
export function installWallets() {
  $("wallet-form").onsubmit = (e) => {
    e.preventDefault();
    busy($("create-wallet"), async () => {
      const name = await createWallet($("wallet-name").value.trim());
      $("wallet-name").value = "";
      toast(`Created ${name}`);
    });
  };
  for (const event of ["pointerdown", "keydown"]) {
    document.addEventListener(event, () => (wallets().lastActive = Date.now()), { passive: true });
  }
  setInterval(() => {
    if (!wallets().idle) return;
    wallets().lock();
    refreshPage();
    toast("Locked after 15 idle minutes");
  }, 30_000);
}
