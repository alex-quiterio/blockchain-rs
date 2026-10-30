// The wallets panel: the vault's lock, the wallets and what each holds, making one, and backing
// them up or restoring them.

import type { Wallet } from "@domain/wallets/wallets";
import { $, el, fmt, rebuildOn, short, showError, toast } from "@ui/dom";
import type { NodePage } from "@ui/node/page";
import { avatar } from "@ui/node/parties";

const lockIcon = (open: boolean): HTMLSpanElement =>
  el("span", { textContent: open ? "🔓" : "🔒", ariaHidden: "true" });

function passphraseInput(
  placeholder: string,
  autocomplete: AutoFill,
): HTMLInputElement {
  return el("input", {
    type: "password",
    placeholder,
    autocomplete,
    required: true,
    minLength: 8,
  });
}

function renderWallets(page: NodePage): void {
  renderVault(page);
  const list = page.wallets.list;
  $("wallet-count").textContent = list.length ? String(list.length) : "";
  $("wallets").replaceChildren(
    ...(list.length
      ? list.map((wallet) => walletView(page, wallet))
      : [
          el("div", {
            className: "empty",
            textContent: `No wallets yet. Make one, then ${page.view.isStake() ? "forge" : "mine"} to fund it.`,
          }),
        ]),
  );
}

function walletView(
  { view, wallets, actions }: NodePage,
  wallet: Wallet,
): HTMLDivElement {
  const account = view.state.accounts[wallet.address] ?? {
    balance: 0,
    staked: 0,
    next_nonce: 0,
  };
  const copy = el("button", {
    className: "icon",
    type: "button",
    textContent: "⧉",
    title: "Copy address",
  });
  copy.onclick = () =>
    navigator.clipboard
      .writeText(wallet.address)
      .then(() => toast("Address copied"));
  const forget = el("button", {
    className: "icon forget",
    type: "button",
    textContent: "✕",
    title: "Forget this wallet",
  });
  forget.onclick = () => {
    if (wallets.locked)
      return toast("Unlock your wallets first", { error: true });
    if (
      !confirm(
        `Forget ${wallet.name}? Its secret key will be gone from this browser.`,
      )
    )
      return;
    wallets.remove(wallet).then(() => actions.refreshPage(), showError);
  };
  return el(
    "div",
    { className: "wallet" },
    avatar(wallet.address),
    el(
      "div",
      { className: "who" },
      el("div", { className: "name", textContent: wallet.name }),
      el(
        "div",
        { className: "addr", title: wallet.address },
        short(wallet.address),
        copy,
      ),
    ),
    el(
      "div",
      { className: "money" },
      el("div", {
        className: `balance${account.balance ? "" : " zero"}`,
        textContent: fmt(account.balance),
      }),
      account.staked
        ? el("div", {
            className: "staked",
            textContent: `${fmt(account.staked)} staked`,
          })
        : el("div", {
            className: "nonce",
            textContent: `nonce ${account.next_nonce}`,
          }),
    ),
    forget,
  );
}

/**
 * The vault's lock: setting a passphrase, unlocking, or what an unlocked vault offers. Rebuilt
 * only when which of those it is changes, so a passphrase being typed survives a redraw.
 */
function renderVault(page: NodePage): void {
  const w = page.wallets;
  $("wallet-form").hidden = w.locked;
  const [key, build] = w.needsPassphrase
    ? [`set-up:${w.legacy.length}`, () => setUpView(page)]
    : w.locked
      ? ["locked", () => unlockView(page)]
      : ["unlocked", () => unlockedView(page)];
  rebuildOn($("vault"), key, build);
}

function unlockedView(page: NodePage): HTMLElement[] {
  const { wallets: w, actions } = page;

  const lock = el("button", {
    className: "small",
    type: "button",
    textContent: "Lock",
  });
  lock.onclick = () => {
    w.lock();
    void actions.refreshPage();
    toast("Locked. Your keys are wiped from memory.");
  };
  const backup = el("button", {
    className: "small",
    type: "button",
    textContent: "Back up",
  });
  backup.onclick = () => downloadBackup(page);
  const restore = el("button", {
    className: "small",
    type: "button",
    textContent: "Restore",
  });
  restore.onclick = () => restoreBackup(page);
  return [
    el(
      "div",
      { className: "unlocked" },
      lockIcon(true),
      el("span", { className: "title", textContent: "Unlocked" }),
      el("span", { className: "actions" }, lock, backup, restore),
    ),
    el("div", {
      className: "unlocked-note",
      textContent:
        "Keys are in this tab's memory until you lock, close the tab or stay idle for 15 minutes.",
    }),
  ];
}

function setUpView({ wallets, actions }: NodePage): HTMLElement[] {
  const first = passphraseInput("Passphrase, 8+ characters", "new-password");
  const again = passphraseInput("Type it again", "new-password");
  const button = el("button", { textContent: "Set passphrase" });
  const form = el("form", {}, first, again, button);
  form.onsubmit = (e) => {
    e.preventDefault();
    if (first.value !== again.value)
      return toast("The passphrases don't match", { error: true });
    void actions.busy(button, async () => {
      button.textContent = "Encrypting…";
      await wallets
        .setUp(first.value)
        .finally(() => (button.textContent = "Set passphrase"));
      toast("Your keys are now encrypted with your passphrase");
    });
  };
  const n = wallets.legacy.length;
  return [
    el(
      "div",
      { className: "lockcard locked" },
      el(
        "div",
        { className: "title" },
        lockIcon(false),
        n
          ? `Protect your ${n} wallet${n === 1 ? "" : "s"}`
          : "Choose a passphrase",
      ),
      el("p", {
        textContent: n
          ? "They're saved unencrypted from an older version. A passphrase encrypts them; the plain copy is then deleted."
          : "It encrypts your wallets' secret keys before they're saved in this browser. There's no way to reset it, so pick one you'll remember.",
      }),
      form,
    ),
  ];
}

function unlockView({ wallets, actions }: NodePage): HTMLElement[] {
  const input = passphraseInput("Passphrase", "current-password");
  input.id = "unlock-passphrase";
  const button = el("button", { textContent: "Unlock" });
  const form = el(
    "form",
    {},
    el(
      "div",
      { className: "row" },
      el("span", { className: "grow" }, input),
      button,
    ),
  );
  form.onsubmit = (e) => {
    e.preventDefault();
    void actions.busy(button, async () => {
      button.textContent = "Unlocking…";
      await wallets
        .unlock(input.value)
        .finally(() => (button.textContent = "Unlock"));
      toast("Unlocked");
    });
  };
  return [
    el(
      "div",
      { className: "lockcard locked" },
      el("div", { className: "title" }, lockIcon(false), "Wallets locked"),
      el("p", {
        textContent:
          "Balances and mining work while locked. Unlock to send, forge or make wallets.",
      }),
      form,
    ),
  ];
}

function downloadBackup({ wallets }: NodePage): void {
  const link = el("a", {
    href: URL.createObjectURL(
      new Blob([wallets.backup()], { type: "application/json" }),
    ),
    download: `blockchain-rs-wallets-${new Date().toISOString().slice(0, 10)}.json`,
  });
  link.click();
  URL.revokeObjectURL(link.href);
  toast(
    "Saved an encrypted backup. You'll need the same passphrase to restore it.",
  );
}

function restoreBackup({ wallets, actions }: NodePage): void {
  const picker = el("input", {
    type: "file",
    accept: "application/json,.json",
  });
  picker.onchange = async () => {
    try {
      const file = picker.files?.[0];
      if (!file) return;
      const text = await file.text();
      const passphrase = prompt("Passphrase this backup was made with:");
      if (!passphrase) return;
      const added = await wallets.restore(text, passphrase);
      toast(
        added
          ? `Restored ${added} wallet${added === 1 ? "" : "s"}`
          : "Those wallets are already here",
      );
      void actions.refreshPage();
    } catch (err) {
      showError(err);
    }
  };
  picker.click();
}

/**
 * Wires up making a wallet, and locking the wallets after a stretch without activity; and draws
 * the wallets each time the view changes.
 */
export function installWallets(page: NodePage): void {
  const { wallets, commands, actions } = page;
  page.view.onChange(() => renderWallets(page));
  $<HTMLFormElement>("wallet-form").onsubmit = (e) => {
    e.preventDefault();
    void actions.busy($<HTMLButtonElement>("create-wallet"), async () => {
      const name = await commands.createWallet(
        $<HTMLInputElement>("wallet-name").value.trim(),
      );
      $<HTMLInputElement>("wallet-name").value = "";
      toast(`Created ${name}`);
    });
  };
  for (const event of ["pointerdown", "keydown"]) {
    document.addEventListener(
      event,
      () => {
        wallets.lastActive = Date.now();
      },
      { passive: true },
    );
  }
  setInterval(() => {
    if (!wallets.idle) return;
    wallets.lock();
    void actions.refreshPage();
    toast("Locked after 15 idle minutes");
  }, 30_000);
}
