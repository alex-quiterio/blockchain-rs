// The wallet page. Shared helpers (api, el, sha256, Merkle, …) live in common.js.

const POLL_MS = 4000;

// ---- Keys: made and used in this page, never sent anywhere ---------------------

const Keys = {
  // A PKCS#8 wrapper around a 32-byte ed25519 seed, the form WebCrypto imports private keys in.
  PKCS8_PREFIX: "302e020100300506032b657004220420",
  ALGORITHM: { name: "Ed25519" },

  async generate() {
    const pair = await crypto.subtle.generateKey(this.ALGORITHM, true, ["sign", "verify"]);
    const publicKey = new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey));
    const pkcs8 = new Uint8Array(await crypto.subtle.exportKey("pkcs8", pair.privateKey));
    return { address: toHex(publicKey), secret_key: toHex(pkcs8.slice(-32)) };
  },

  async sign(secretKey, message) {
    const key = await crypto.subtle.importKey(
      "pkcs8", fromHex(this.PKCS8_PREFIX + secretKey), this.ALGORITHM, false, ["sign"]);
    const bytes = new TextEncoder().encode(message);
    return toHex(new Uint8Array(await crypto.subtle.sign(this.ALGORITHM, key, bytes)));
  },

  /**
   * A transaction signed exactly as the node expects: `from|to|amount|nonce`, then `|fee=N` and
   * `|commit=C` when there's a fee or a commitment.
   */
  async transaction(wallet, to, amount, nonce, fee, commitment) {
    const extras = `${fee ? `|fee=${fee}` : ""}${commitment ? `|commit=${commitment}` : ""}`;
    const payload = `${wallet.address}|${to}|${amount}|${nonce}${extras}`;
    const signature = await this.sign(Wallets.secretOf(wallet.address), payload);
    const tx = { from: wallet.address, to, amount, nonce, signature };
    return { ...tx, ...(fee ? { fee } : {}), ...(commitment ? { commitment } : {}) };
  },
};

// ---- Validator randomness: a hash chain committed to once, revealed a link per block -----

const Randao = {
  LENGTH: 1000, // must match randao::LENGTH on the node

  async hashTimes(value, times) {
    for (let i = 0; i < times; i++) value = await sha256(value);
    return value;
  },

  /** The secret start of commitment number `epoch`, derived from the key so nothing extra is stored. */
  seed(secretKey, epoch) {
    return sha256(`blockchain-rs/randao|${secretKey}|${epoch}`);
  },

  /** What a first stake publishes: the seed hashed LENGTH times. */
  async commitment(secretKey, epoch) {
    return this.hashTimes(await this.seed(secretKey, epoch), this.LENGTH);
  },

  /** The link to reveal next, with `remaining` reveals left: one hash short of the last one shown. */
  async reveal(secretKey, epoch, remaining) {
    return this.hashTimes(await this.seed(secretKey, epoch), remaining - 1);
  },
};


// ---- Vault: secret keys encrypted with a passphrase before they're saved -----------

const Vault = {
  // OWASP's 2023 recommendation for PBKDF2-SHA256: slow enough to make guessing expensive.
  ITERATIONS: 600_000,

  async deriveKey(passphrase, salt, iterations) {
    const material = await crypto.subtle.importKey(
      "raw", new TextEncoder().encode(passphrase), "PBKDF2", false, ["deriveKey"]);
    return crypto.subtle.deriveKey(
      { name: "PBKDF2", hash: "SHA-256", salt, iterations },
      material, { name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"]);
  },

  /** Encrypts `secrets` under a session's key, with a fresh IV every time. */
  async seal(secrets, session) {
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const plain = new TextEncoder().encode(JSON.stringify(secrets));
    const data = new Uint8Array(await crypto.subtle.encrypt({ name: "AES-GCM", iv }, session.key, plain));
    return { version: 1, kdf: "PBKDF2-SHA256", iterations: session.iterations, salt: toHex(session.salt), iv: toHex(iv), data: toHex(data) };
  },

  /** Decrypts a sealed vault. AES-GCM refuses a wrong key outright, so a bad passphrase can't half-work. */
  async open(vault, passphrase) {
    const session = { salt: fromHex(vault.salt), iterations: vault.iterations };
    session.key = await this.deriveKey(passphrase, session.salt, session.iterations);
    try {
      const plain = await crypto.subtle.decrypt({ name: "AES-GCM", iv: fromHex(vault.iv) }, session.key, fromHex(vault.data));
      return { secrets: JSON.parse(new TextDecoder().decode(plain)), session };
    } catch {
      throw new Error("Wrong passphrase");
    }
  },
};

// ---- Wallets: names and addresses in the clear, secret keys only in the vault -------

const Wallets = {
  PUBLIC: "blockchain-rs:accounts",
  VAULT: "blockchain-rs:vault",
  LEGACY: "blockchain-rs:wallets", // older versions kept keys here unencrypted
  IDLE_MS: 15 * 60 * 1000,

  list: [],       // [{ name, address }]: public, so balances show even while locked
  vault: null,    // the encrypted secret keys, as saved
  legacy: [],     // unencrypted wallets from older versions, waiting for a passphrase
  secrets: null,  // { address: secret_key }, only in memory and only while unlocked
  session: null,  // the key derived from the passphrase, while unlocked
  lastActive: Date.now(),

  load() {
    const read = (key, fallback) => {
      try { return JSON.parse(localStorage.getItem(key)) ?? fallback; } catch { return fallback; }
    };
    this.list = read(this.PUBLIC, []);
    this.vault = read(this.VAULT, null);
    this.legacy = read(this.LEGACY, []).filter((w) => w.secret_key);
    for (const w of this.legacy) {
      if (!this.byAddress(w.address)) this.list.push({ name: w.name, address: w.address });
    }
  },

  get locked() { return !this.secrets; },
  get needsPassphrase() { return !this.vault; },

  /** First run: choose a passphrase. Any unencrypted wallets from older versions move into the vault. */
  async setUp(passphrase) {
    const salt = crypto.getRandomValues(new Uint8Array(16));
    this.session = { salt, iterations: Vault.ITERATIONS, key: await Vault.deriveKey(passphrase, salt, Vault.ITERATIONS) };
    this.secrets = {};
    await this.adoptLegacy();
  },

  async unlock(passphrase) {
    const { secrets, session } = await Vault.open(this.vault, passphrase);
    Object.assign(this, { secrets, session, lastActive: Date.now() });
    await this.adoptLegacy();
  },

  lock() {
    this.secrets = null;
    this.session = null;
  },

  async adoptLegacy() {
    for (const w of this.legacy) this.secrets[w.address] = w.secret_key;
    await this.save();
    this.legacy = [];
    try { localStorage.removeItem(this.LEGACY); } catch { /* private mode */ }
  },

  async save() {
    this.vault = await Vault.seal(this.secrets, this.session);
    try {
      localStorage.setItem(this.VAULT, JSON.stringify(this.vault));
      localStorage.setItem(this.PUBLIC, JSON.stringify(this.list));
    } catch { /* private mode: the wallets last as long as the tab */ }
  },

  async add({ name, address, secret_key }) {
    this.secretOf(null);
    this.list.push({ name, address });
    this.secrets[address] = secret_key;
    await this.save();
  },

  async remove(wallet) {
    this.secretOf(null);
    this.list = this.list.filter((w) => w.address !== wallet.address);
    delete this.secrets[wallet.address];
    await this.save();
  },

  /** A wallet's secret key. Throws while locked, so nothing can sign without the passphrase. */
  secretOf(address) {
    if (this.locked) throw new Error("Unlock your wallets first");
    this.lastActive = Date.now();
    return address && this.secrets[address];
  },

  byAddress(address) { return this.list.find((w) => w.address === address); },

  /** The encrypted vault plus the public list. Useless without the passphrase. */
  backup() {
    return JSON.stringify({ format: "blockchain-rs-backup", version: 1, accounts: this.list, vault: this.vault }, null, 2);
  },

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
    await this.save();
    return added;
  },
};
Wallets.load();

// ---- Chain state, as last seen ---------------------------------------------

// Only the latest blocks are kept and drawn, so the page costs the same however long the chain
// gets. The explorer shows the rest.
const SHOWN_BLOCKS = 30;

// `blocks` holds the latest few, oldest first; `works` is the required work, by height.
const state = { height: -1, blocks: [], works: {}, pending: [], valid: null, consensus: null, accounts: {}, peers: [], node: null, seenTip: null };

const isStake = () => state.consensus?.kind === "proof_of_stake";
const tip = () => state.blocks.at(-1)?.hash;
// Fees only move coins around; the supply grows by the block reward alone.
const supply = () => Math.max(0, state.height) * (state.consensus?.reward ?? 0);
/** The block at a height, if it's among the ones kept. */
const blockAt = (height) => state.blocks[height - (state.blocks[0]?.index ?? 0)];
/** Enough blocks to draw the shown ones, the block before each, and a retarget window back. */
const kept = () => SHOWN_BLOCKS + 1 + (state.consensus?.retarget_interval ?? 0);

async function load() {
  const [status, pending, consensus, peers, node] = await Promise.all([
    api("GET", "/status"), api("GET", "/transactions/pending"), api("GET", "/consensus"),
    api("GET", "/peers"), state.node || api("GET", "/node"),
  ]);
  Object.assign(state, { pending, consensus, peers, node });
  const forked = await loadBlocks(status);
  // A full replay of the chain: once on opening, and again only if a fork replaced our tip.
  if (state.valid === null || forked) state.valid = (await api("GET", "/chain/valid")).valid;
  const accounts = await Promise.all(Wallets.list.map((w) => api("GET", `/balances/${w.address}`)));
  state.accounts = Object.fromEntries(accounts.map((a) => [a.address, a]));
}

/**
 * Brings the kept blocks up to the node's tip. Usually that's fetching just the new ones; if they
 * don't link to our tip, a fork replaced it, and the latest few are fetched again. Returns
 * whether that happened.
 */
async function loadBlocks(status) {
  if (status.tip === tip()) return false;
  const have = state.blocks.at(-1)?.index ?? -1;
  const window = Math.max(0, status.height + 1 - kept());
  let blocks = null;
  if (have >= 0 && status.height > have && status.height - have < kept()) {
    const fresh = await api("GET", `/chain?from=${have + 1}`);
    if (fresh[0]?.prev_hash === tip()) blocks = state.blocks.concat(fresh).slice(-kept());
  }
  const forked = blocks === null && have >= 0 && status.height - have < kept();
  blocks ??= await api("GET", `/chain?from=${window}`);
  const start = blocks[0]?.index ?? 0;
  // Per-block required work; past 2^53 it's approximate, which is fine for display.
  const works = isStake() ? [] : await api("GET", `/chain/work?from=${start}&limit=${blocks.length}`);
  Object.assign(state, {
    height: status.height,
    blocks,
    works: Object.fromEntries(works.map((w, i) => [start + i, Number(w)])),
  });
  return forked;
}

// ---- View helpers -----------------------------------------------------------

const nameOf = (address) => Wallets.byAddress(address)?.name || short(address);
const pct = (part, whole) => `${whole ? Math.round((part / whole) * 100) : 0}%`;

function hues(address) {
  const h1 = parseInt(address.slice(0, 4), 16) % 360;
  return [h1, (h1 + 40 + (parseInt(address.slice(4, 6), 16) % 120)) % 360];
}

/** A gradient tile derived from the address bytes, so the same address always looks the same. */
function avatar(address, small = false) {
  const [h1, h2] = hues(address);
  const tile = el("span", { className: `avatar${small ? " sm" : ""}` });
  tile.style.background = `linear-gradient(135deg, hsl(${h1} 75% 58%), hsl(${h2} 70% 42%))`;
  return tile;
}

/** The leading hex zeros a given amount of work guarantees: 16^d hashes means d zeros. */
const zerosFor = (work) => Math.max(0, Math.floor(Math.log(work) / Math.log(16) + 1e-9));
/** "65,536", or "4.3M" once it gets long. */
const fmtWork = (work) => (work >= 1e6
  ? new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 }).format(work)
  : fmt(work));

function hashView(hash, work) {
  const zeros = hash.match(/^0*/)[0].length;
  const shown = isStake() || !work ? 0 : Math.min(zeros, zerosFor(work));
  return el("div", { className: "hash", title: hash },
    el("span", { className: "zeros", textContent: hash.slice(0, shown) }), hash.slice(shown));
}

function toast(message, isError = false) {
  const note = el("div", { className: `toast${isError ? " error" : ""}`, textContent: message });
  $("toasts").append(note);
  setTimeout(() => note.remove(), 3800);
}

function explain(question, ...items) {
  return el("details", { className: "explain" }, el("summary", { textContent: question }),
    el("ol", {}, items.map((html) => el("li", { innerHTML: html }))));
}

// ---- Components -------------------------------------------------------------

const party = (address) =>
  el("span", { className: "party" }, avatar(address, true), el("span", { textContent: nameOf(address) }));
const arrow = () => el("span", { className: "arrow", textContent: "→" });
const feeNote = (tx) => (tx.fee ? el("span", { className: "fee", textContent: `fee ${fmt(tx.fee)}` }) : null);

function txView(tx) {
  const kind = kindOf(tx);
  if (kind === "coinbase") {
    return el("li", { className: "tx coinbase" },
      el("span", { className: "tag", textContent: "payout" }), arrow(), party(tx.to),
      el("span", { className: "amt", textContent: `+${fmt(tx.amount)}` }));
  }
  if (kind === "stake" || kind === "unstake") {
    return el("li", { className: "tx", title: `nonce ${tx.nonce}` },
      party(tx.from), el("span", { className: `tag ${kind}`, textContent: kind === "stake" ? "stakes" : "unstakes" }),
      feeNote(tx), el("span", { className: "amt", textContent: fmt(tx.amount) }));
  }
  return el("li", { className: "tx", title: `nonce ${tx.nonce}` },
    party(tx.from), arrow(), party(tx.to), feeNote(tx), el("span", { className: "amt", textContent: fmt(tx.amount) }));
}

function walletView(wallet) {
  const account = state.accounts[wallet.address] || { balance: 0, staked: 0, next_nonce: 0 };
  const copy = el("button", { className: "icon", type: "button", textContent: "⧉", title: "Copy address" });
  copy.onclick = () => navigator.clipboard.writeText(wallet.address).then(() => toast("Address copied"));
  const forget = el("button", { className: "icon forget", type: "button", textContent: "✕", title: "Forget this wallet" });
  forget.onclick = () => {
    if (Wallets.locked) return toast("Unlock your wallets first", true);
    if (!confirm(`Forget ${wallet.name}? Its secret key will be gone from this browser.`)) return;
    Wallets.remove(wallet).then(refresh, (err) => toast(err.message, true));
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

function blockView(block, prev, fresh) {
  const genesis = block.index === 0;
  const [payout, ...rest] = block.transactions;
  const forged = Boolean(block.validator);
  return el("li", { className: `block${genesis ? " genesis" : ""}${fresh ? " fresh" : ""}` },
    el("div", { className: "marker", textContent: block.index }),
    el("article", { className: "card" },
      el("div", { className: "card-head" },
        el("strong", { textContent: genesis ? "Genesis" : `Block ${block.index}` }),
        genesis ? null : el("span", { className: "when", textContent: ago(block.timestamp), title: new Date(block.timestamp).toLocaleString() }),
        genesis ? null : el("span", { className: `badge-kind ${forged ? "pos" : "pow"}`, textContent: forged ? "forged" : "mined" }),
        el("span", { className: "ntx", textContent: `${block.transactions.length} tx` })),
      hashView(block.hash, state.works?.[block.index]),
      el("div", { className: "meta" },
        forged
          ? el("span", {}, "forged by ", party(block.validator))
          : el("span", {}, "nonce ", el("span", { className: "mono", textContent: fmt(block.nonce) })),
        el("span", {}, "prev ", el("span", { className: "mono", textContent: short(block.prev_hash) }))),
      txList(block.transactions),
      breakdown(block, prev, payout, rest)));
}

const SHOWN_TXS = 6;

/** A block's transactions; long lists show the first few and fold the rest away. */
function txList(transactions) {
  if (!transactions.length) return null;
  const list = el("ul", { className: "txs" }, transactions.slice(0, SHOWN_TXS).map(txView));
  if (transactions.length > SHOWN_TXS) {
    const rest = transactions.slice(SHOWN_TXS);
    const more = el("details", { className: "more-txs" },
      el("summary", { textContent: `${fmt(rest.length)} more transactions · ${fmt(feesOf(rest))} in fees` }),
      el("ul", { className: "txs" }, rest.map(txView)));
    list.append(el("li", {}, more));
  }
  return list;
}

/** The "why is this block valid" story, one fact per row. */
function breakdown(block, prev, payout, transfers) {
  const facts = [];
  const row = (label, ...value) => facts.push(el("dt", { textContent: label }), el("dd", {}, value));
  const yes = (text) => el("span", { className: "ok", textContent: `✓ ${text}` });

  if (block.index === 0) {
    row("Role", "The fixed first block. Every node with the same rules builds the same one, so they all start from the same place.");
  } else {
    row("Link", yes("prev matches"), ` block ${prev.index}'s hash, so changing block ${prev.index} would break this one too.`);
  }
  if (block.validator) {
    row("Proof", yes("signed"), " by ", party(block.validator),
      ". The validator was drawn at random, weighted by stake, from a randomness beacon that only validators' committed reveals can move, so every node agrees whose turn it was and nobody could steer it.");
    if (block.reveal) {
      row("Randomness", "revealed ", el("span", { className: "formula", textContent: short(block.reveal) }),
        ", the next link of a hash chain the validator committed to when staking. It was fixed in advance, so they had nothing to choose; it's mixed into the beacon that picked the next validator.");
    }
  } else {
    const work = state.works[block.index];
    row("Proof", yes(`below the target for ${fmtWork(work)} hashes`),
      ` Found on try ${fmt(block.nonce + 1)}. Each try changes the nonce and rehashes; at this difficulty it takes ${fmtWork(work)} tries on average.`);
    const before = state.works[block.index - 1];
    if (block.index > 0 && before && work !== before) {
      const { target_block_secs: target, retarget_interval: n } = state.consensus;
      const [last, first] = [blockAt(block.index - 1), blockAt(block.index - 1 - n)];
      const took = last && first ? (new Date(last.timestamp) - new Date(first.timestamp)) / 1000 : NaN;
      if (!Number.isNaN(took)) row("Difficulty", work > before ? "↑ " : "↓ ", el("b", { textContent: `${(work / before).toFixed(2)}×` }),
        `: the last ${n} blocks took ${took.toFixed(0)}s against a target of ${target * n}s, so ${work > before ? "mining got harder" : "mining got easier"} (by at most 4×).`);
    }
  }
  if (payout) {
    const fees = feesOf(transfers);
    row("Payout", `${fmt(payout.amount - fees)} reward + ${fmt(fees)} in fees = `, el("b", { textContent: fmt(payout.amount) }), " to ", party(payout.to));
  }
  const check = el("span", { textContent: "checking…" });
  Merkle.root(block.transactions).then((root) => check.replaceWith(root === block.merkle_root
    ? yes("match the Merkle root") : el("span", { textContent: "✗ don't match the Merkle root" })));
  const n = block.transactions.length;
  row("Contents", check, " ", el("span", { className: "formula", textContent: short(block.merkle_root) }),
    n > 1
      ? `, recomputed here by hashing the ${n} transactions in pairs up to one hash. The block hash covers the root, so changing any transaction breaks the block, and anyone holding just the header can check one transaction with ${Math.ceil(Math.log2(n))} hashes.`
      : ", recomputed here from the transactions. The block hash covers the root, so changing any transaction breaks the block.");
  const fields = ["index", "timestamp", "prev_hash", "merkle_root", "nonce", block.validator && "validator"].filter(Boolean);
  row("Hash", el("span", { className: "formula", textContent: `sha256(${fields.join(" | ")})` }));
  return el("details", { className: "breakdown" }, el("summary", { textContent: "Breakdown" }), el("dl", { className: "facts" }, facts));
}

// ---- Panels -----------------------------------------------------------------

function renderStats() {
  const c = state.consensus;
  $("stat-consensus").textContent = c ? (isStake() ? "Proof of stake" : `Proof of work · ${fmtWork(Number(c.work))}`) : "–";
  $("stat-height").textContent = state.height >= 0 ? fmt(state.height) : "–";
  $("stat-supply").textContent = fmt(supply());
  $("stat-pending").textContent = fmt(state.pending.length);
  const pill = $("validity");
  pill.className = `pill ${state.valid ? "ok" : "bad"}`;
  pill.lastChild.textContent = state.valid ? "chain valid" : "chain INVALID";
}

const lockIcon = (open) => el("span", { textContent: open ? "🔓" : "🔒", ariaHidden: "true" });

function passphraseInput(placeholder, autocomplete) {
  return el("input", { type: "password", placeholder, autocomplete, required: true, minLength: 8 });
}

function renderVault() {
  $("wallet-form").hidden = Wallets.locked;
  if (Wallets.needsPassphrase) return renderSetUp();
  if (Wallets.locked) return renderUnlock();

  const lock = el("button", { className: "small", type: "button", textContent: "Lock" });
  lock.onclick = () => { Wallets.lock(); refresh(); toast("Locked. Your keys are wiped from memory."); };
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
    if (first.value !== again.value) return toast("The passphrases don't match", true);
    busy(button, async () => {
      button.textContent = "Encrypting…";
      await Wallets.setUp(first.value);
      toast("Your keys are now encrypted with your passphrase");
    });
  };
  const n = Wallets.legacy.length;
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
      await Wallets.unlock(input.value);
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
    href: URL.createObjectURL(new Blob([Wallets.backup()], { type: "application/json" })),
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
      const added = await Wallets.restore(text, passphrase);
      toast(added ? `Restored ${added} wallet${added === 1 ? "" : "s"}` : "Those wallets are already here");
      refresh();
    } catch (err) {
      toast(err.message, true);
    }
  };
  picker.click();
}

function renderWallets() {
  renderVault();
  $("wallet-count").textContent = Wallets.list.length || "";
  $("wallets").replaceChildren(
    ...(Wallets.list.length
      ? Wallets.list.map(walletView)
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

function fillSelect(select, extras = []) {
  const current = select.value;
  select.replaceChildren(
    ...Wallets.list.map((w) => el("option", { value: w.address, textContent: w.name })),
    ...extras.map(([value, label]) => el("option", { value, textContent: label })),
  );
  if ([...select.options].some((o) => o.value === current)) select.value = current;
}

function renderProducer() {
  if (isStake()) return renderForge();
  $("producer-title").textContent = "Mine";
  const select = el("select", { id: "miner" });
  fillSelect(select);
  const button = el("button", { className: "gold", textContent: "Mine block" });
  const form = el("form", { className: "inline" }, el("label", { className: "grow" }, "Reward to ", select), button);
  form.onsubmit = (e) => { e.preventDefault(); mine(button, select.value); };
  const c = state.consensus;
  const work = Number(c.work);
  const pace = c.target_block_secs
    ? el("p", { className: "summary pace" },
      "Next block: about ", el("b", { textContent: fmtWork(work) }), " hashes. Aiming for one block every ",
      el("b", { textContent: `${c.target_block_secs}s` }), `; the next adjustment is at block ${fmt(c.next_retarget)}.`)
    : el("p", { className: "summary pace" }, "Next block: about ", el("b", { textContent: fmtWork(work) }), " hashes, at a fixed difficulty.");
  $("producer").replaceChildren(form, pace, explain("How does proof of work work?",
    `The miner bundles the mempool and a payout to themselves into a block, then hashes it again and again, changing the <code>nonce</code> each time.`,
    `The block counts only once its hash, read as a number, is below a target. Right now that takes about ${fmtWork(work)} tries, so rewriting history means redoing all that work.`,
    c.target_block_secs
      ? `The target moves to keep blocks coming every ${c.target_block_secs}s. Every ${c.retarget_interval} blocks the node checks how long they took: faster means harder, slower means easier, by at most 4× at a time. Timestamps must be after the median of the last 11 and at most 2 hours ahead, so miners can't fake the pace.`
      : `The difficulty is fixed on this node. Start it with <code>TARGET_BLOCK_SECS</code> above 0 to let it adjust.`,
    `When forks disagree, the chain with the most total work wins, not the longest. Exact ties go to the lower tip hash, so every node picks the same one.`));
}

function renderForge() {
  $("producer-title").textContent = "Forge";
  const { next_validator: next, stakes, total_stake: total } = state.consensus;
  const mine = next && Wallets.byAddress(next);
  const children = [];

  if (!next) {
    const select = el("select");
    fillSelect(select);
    const button = el("button", { className: "stake", textContent: "Forge block" });
    const form = el("form", { className: "inline" }, el("label", { className: "grow" }, "Forge as ", select), button);
    form.onsubmit = (e) => { e.preventDefault(); forge(button, Wallets.byAddress(select.value)); };
    children.push(el("div", { className: "turn" }, el("div", {},
      el("div", { className: "who", textContent: "Anyone's turn" }),
      el("div", { className: "sub", textContent: "Nobody has staked yet, so any wallet may forge. Forge a few blocks, then stake some coins." }))), form);
  } else {
    const button = el("button", { className: "stake", textContent: mine ? `Forge as ${mine.name}` : "Not your turn", disabled: !mine });
    button.onclick = () => forge(button, mine);
    children.push(el("div", { className: "turn" }, avatar(next), el("div", { className: "grow" },
      el("div", { className: "who", textContent: `${nameOf(next)}'s turn` }),
      el("div", { className: "sub", textContent: mine ? "Their key is in this browser." : "Their key isn't in this browser, so only they can forge." }))), button);
  }

  if (stakes.length) {
    children.push(el("h3", { textContent: `Stake · ${fmt(total)} locked` }),
      el("div", { className: "stakebar" }, stakes.map((s) => {
        const seg = el("span", { title: `${nameOf(s.address)}: ${fmt(s.amount)}` });
        seg.style.width = pct(s.amount, total);
        const [from, to] = hues(s.address);
        seg.style.background = `linear-gradient(90deg, hsl(${from} 75% 58%), hsl(${to} 70% 42%))`;
        return seg;
      })),
      el("ul", { className: "legend" }, stakes.map((s) => el("li", { className: s.address === next ? "next" : "" },
        party(s.address), el("span", { className: "fee", textContent: fmt(s.amount) }),
        el("span", { className: "pct", textContent: `${pct(s.amount, total)} chance` })))));
  }

  children.push(explain("How does proof of stake work?",
    `Validators lock coins by sending them to <code>stake</code>. Locked coins can't be spent until they're unstaked.`,
    `For each block, one validator is drawn at random, weighted by stake, from a randomness <b>beacon</b>. Every node computes the same draw.`,
    `When you first stake, this page commits to the end of a secret hash chain made from your key. Each block you forge must reveal the next link back, which anyone can check by hashing it once.`,
    `Reveals are fixed the moment you commit, so a forger can't try different timestamps or contents to make the draw pick them again. Each reveal is mixed into the beacon, which picks the next validator.`,
    `That validator signs the block instead of doing work. Anyone can check the signature, and a block from the wrong validator is rejected.`,
    `The validator earns the reward plus the block's fees, so more stake means more turns and more income.`,
    `When forks disagree, the one with the most stake behind its blocks wins, not the longest. A long chain nobody staked on can't outweigh a short one backed by real stake.`));
  $("producer").replaceChildren(...children);
}

function renderPending() {
  const fees = feesOf(state.pending);
  $("pending-fees").textContent = fees ? `· ${fmt(fees)} in fees for the next block` : "";
  $("pending").replaceChildren(
    ...(state.pending.length
      ? state.pending.map(txView)
      : [el("li", { className: "empty", textContent: "Nothing waiting to be added." })]),
  );
}

function renderChain() {
  const fresh = state.seenTip !== null && tip() !== state.seenTip;
  state.seenTip = tip();
  const open = new Set([...document.querySelectorAll(".breakdown[open]")].map((d) => d.dataset.hash));
  const shown = state.blocks.slice(-SHOWN_BLOCKS).reverse();
  const older = shown.at(-1)?.index ?? 0;
  $("blocks").replaceChildren(
    ...shown.map((block, i) => {
      const view = blockView(block, blockAt(block.index - 1), fresh && i === 0);
      const details = view.querySelector(".breakdown");
      details.dataset.hash = block.hash;
      details.open = open.has(block.hash);
      return view;
    }),
    older > 0 ? el("li", { className: "older" },
      el("a", { href: `/explorer.html#/block/${older - 1}`, textContent: `${fmt(older)} older blocks in the Explorer →` })) : null,
  );
}

function updateSendSummary() {
  const to = $("send-to").value;
  const amount = Number($("send-amount").value) || 0;
  const fee = Number($("send-fee").value) || 0;
  const from = state.accounts[$("send-from").value];
  const what = to === "__stake" ? "locks" : to === "__unstake" ? "unlocks" : "sends";
  const cost = to === "__unstake" ? fee : amount + fee;
  const parts = [`${what} `, el("b", { textContent: fmt(amount) })];
  if (fee) parts.push(" + ", el("b", { textContent: fmt(fee) }), " fee");
  parts.push(` · costs ${fmt(cost)} of `, el("b", { textContent: fmt(from?.balance ?? 0) }));
  $("send-summary").replaceChildren(...parts);
}

function renderPeers() {
  $("peer-count").textContent = state.peers.length || "";
  const ours = state.height + 1;
  $("peers").replaceChildren(
    ...(state.peers.length
      ? state.peers.map((p) => {
        const known = p.height != null;
        const diff = known ? p.height - ours : 0;
        const rel = !known ? "not reached yet" : diff > 0 ? `${diff} ahead` : diff < 0 ? `${-diff} behind` : "in step";
        return el("li", { className: "peer" },
          el("span", { className: `dot ${p.error ? "down" : known ? "up" : ""}` }),
          el("span", { className: "url", textContent: p.url, title: p.url }),
          el("span", { className: `rel${diff > 0 ? " ahead" : ""}`, textContent: rel }),
          p.error
            ? el("span", { className: "note err", textContent: p.error })
            : p.last_seen ? el("span", { className: "note", textContent: `height ${p.height - 1} · seen ${ago(p.last_seen)}` }) : null);
      })
      : [el("li", { className: "empty", textContent: "No peers. This node is on its own. Add another node's URL to sync with it." })]),
  );
}

function renderFooter() {
  const n = state.node;
  if (!n) return;
  const dirty = n.commit.endsWith("-dirty");
  $("footer").replaceChildren(
    el("span", {}, "node ", el("span", { className: "mono", textContent: `v${n.version}` })),
    "·",
    el("span", { title: dirty ? "built with uncommitted changes" : "" }, "commit ",
      el("span", { className: `mono${dirty ? " dirty" : ""}`, textContent: n.commit })),
    "·",
    el("span", { textContent: `up since ${new Date(n.started_at).toLocaleString()}` }),
  );
}

async function refresh() {
  try {
    await load();
    renderStats();
    renderPeers();
    renderFooter();
    renderWallets();
    renderProducer();
    renderPending();
    renderChain();
  } catch (err) {
    toast(err.message, true);
  }
}

// ---- Actions ----------------------------------------------------------------

let working = false;
async function busy(button, action) {
  button.disabled = working = true;
  try {
    await action();
    await refresh();
  } catch (err) {
    toast(err.message, true);
  } finally {
    button.disabled = working = false;
  }
}

function showProducing(text) {
  const status = $("producing");
  const started = performance.now();
  const tick = () => status.replaceChildren(el("span", { className: "spin" }),
    el("span", {}, text, ` · ${((performance.now() - started) / 1000).toFixed(1)}s`));
  tick();
  status.hidden = false;
  const timer = setInterval(tick, 100);
  return (message) => {
    clearInterval(timer);
    if (!message) return (status.hidden = true);
    status.replaceChildren(el("span", {}, message, ` in ${((performance.now() - started) / 1000).toFixed(2)}s`));
    setTimeout(() => (status.hidden = true), 5000);
  };
}

function mine(button, miner) {
  if (!miner) return toast("Create a wallet first", true);
  const done = showProducing(`Trying about ${fmtWork(Number(state.consensus.work))} hashes`);
  busy(button, async () => {
    try {
      const block = await api("POST", "/mine", { miner });
      done(`Block ${block.index} found on try ${fmt(block.nonce + 1)}`);
    } catch (err) {
      done();
      throw err;
    }
  });
}

/** Stops an action that needs a key while the wallets are locked, pointing at the unlock box. */
function requireUnlocked() {
  if (!Wallets.locked) return;
  $("unlock-passphrase")?.focus();
  throw new Error("Unlock your wallets first");
}

function forge(button, wallet) {
  if (!wallet) return toast("Create a wallet first", true);
  busy(button, async () => {
    requireUnlocked();
    // The node proposes, this page signs, and only the signature and the reveal go back.
    const secret = Wallets.secretOf(wallet.address);
    const account = await api("GET", `/balances/${wallet.address}`);
    const due = state.consensus.next_validator && account.reveals_left;
    const reveal = due ? await Randao.reveal(secret, account.next_epoch - 1, account.reveals_left) : undefined;
    const block = await api("POST", "/forge/proposal", { validator: wallet.address, reveal });
    block.signature = await Keys.sign(secret, block.hash);
    await api("POST", "/blocks", block);
    toast(`${wallet.name} forged block ${block.index}`);
  });
}

$("peer-form").onsubmit = (e) => {
  e.preventDefault();
  busy($("add-peer"), async () => {
    const url = $("peer-url").value.trim();
    if (!url) return;
    const { added } = await api("POST", "/peers", { url });
    $("peer-url").value = "";
    toast(added ? `Added ${url}. It now knows this node too.` : `${url} is already a peer`);
  });
};

$("sync").onclick = (e) =>
  busy(e.target, async () => {
    const report = await api("POST", "/sync");
    toast(report.adopted_from
      ? `Adopted the heavier chain from ${report.adopted_from}`
      : `Reached ${report.reached} peer${report.reached === 1 ? "" : "s"}. This chain is already the heaviest.`);
  });

$("wallet-form").onsubmit = (e) => {
  e.preventDefault();
  busy($("create-wallet"), async () => {
    const { address, secret_key } = await Keys.generate();
    const name = $("wallet-name").value.trim() || `Wallet ${Wallets.list.length + 1}`;
    await Wallets.add({ name, address, secret_key });
    $("wallet-name").value = "";
    toast(`Created ${name}`);
  });
};

for (const id of ["send-from", "send-to", "send-amount", "send-fee"]) $(id).oninput = updateSendSummary;

$("send-form").onsubmit = (e) => {
  e.preventDefault();
  busy($("send"), async () => {
    const from = Wallets.byAddress($("send-from").value);
    if (!from) throw new Error("Create a wallet first");
    requireUnlocked();
    let to = $("send-to").value;
    if (to === "__stake") to = STAKE;
    else if (to === "__unstake") to = UNSTAKE;
    else if (to === "__other") to = (prompt("Recipient address (hex)") || "").trim();
    if (!to) return;
    const amount = Number($("send-amount").value);
    const fee = Number($("send-fee").value) || 0;
    const { next_nonce, commitment_due } = await api("GET", `/balances/${from.address}`);
    // A first stake commits to the reveal chain this wallet will forge with.
    const commitment = to === STAKE && commitment_due != null
      ? await Randao.commitment(Wallets.secretOf(from.address), commitment_due)
      : undefined;
    const tx = await Keys.transaction(from, to, amount, next_nonce, fee, commitment);
    await api("POST", "/transactions", tx);
    toast(`Queued from ${from.name}. It confirms in the next block.`);
  });
};

// Wipe keys from memory after a stretch without activity.
for (const event of ["pointerdown", "keydown"]) {
  document.addEventListener(event, () => (Wallets.lastActive = Date.now()), { passive: true });
}
setInterval(() => {
  if (!Wallets.locked && Date.now() - Wallets.lastActive > Wallets.IDLE_MS) {
    Wallets.lock();
    refresh();
    toast("Locked after 15 idle minutes");
  }
}, 30_000);

// Pick up changes made elsewhere: other tabs, other nodes syncing, `POST /chain`. Only the
// node's status is polled; blocks are fetched when its tip moves.
setInterval(async () => {
  if (document.hidden || working) return;
  try {
    const [status, peers] = await Promise.all([api("GET", "/status"), api("GET", "/peers")]);
    if (status.tip !== tip() || status.pending !== state.pending.length) return refresh();
    state.peers = peers;
    renderPeers();
  } catch { /* node down; the next tick tries again */ }
}, POLL_MS);

refresh();
