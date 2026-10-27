const $ = (id) => document.getElementById(id);
const POLL_MS = 4000;

// ---- Node API ---------------------------------------------------------------

async function api(method, path, body) {
  const res = await fetch(path, {
    method,
    headers: body ? { "content-type": "application/json" } : {},
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => null);
  if (!res.ok) throw new Error(data?.message || `${res.status} ${res.statusText}`);
  return data;
}

// ---- Wallets kept in this browser ------------------------------------------

const Wallets = {
  KEY: "blockchain-rs:wallets",
  list: (() => {
    try { return JSON.parse(localStorage.getItem("blockchain-rs:wallets") || "[]"); } catch { return []; }
  })(),
  save() {
    try { localStorage.setItem(this.KEY, JSON.stringify(this.list)); } catch { /* private mode */ }
  },
  add(wallet) { this.list.push(wallet); this.save(); },
  remove(wallet) { this.list = this.list.filter((w) => w !== wallet); this.save(); },
  byAddress(address) { return this.list.find((w) => w.address === address); },
};

// ---- Chain state, as last seen ---------------------------------------------

const state = { blocks: [], pending: [], valid: null, tip: null, seenTip: null };

const leadingZeros = (hash) => hash.match(/^0*/)[0].length;
// Every block meets the difficulty, so the fewest leading zeros on any block is our best guess at it.
const difficulty = () => (state.blocks.length ? Math.min(...state.blocks.map((b) => leadingZeros(b.hash))) : 0);
const supply = () =>
  state.blocks.flatMap((b) => b.transactions).filter((t) => t.from === "coinbase").reduce((n, t) => n + t.amount, 0);

async function load() {
  const [blocks, pending, validity] = await Promise.all([
    api("GET", "/chain"), api("GET", "/transactions/pending"), api("GET", "/chain/valid"),
  ]);
  Object.assign(state, { blocks, pending, valid: validity.valid, tip: blocks.at(-1)?.hash });
}

// ---- Small view helpers -----------------------------------------------------

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  Object.assign(node, attrs);
  node.append(...children.filter((c) => c != null));
  return node;
}

const short = (hex) => (hex.length > 16 ? `${hex.slice(0, 8)}…${hex.slice(-6)}` : hex);
const nameOf = (address) => Wallets.byAddress(address)?.name || short(address);
const fmt = (n) => n.toLocaleString();

/** A gradient tile derived from the address bytes, so the same address always looks the same. */
function avatar(address, small = false) {
  const h1 = parseInt(address.slice(0, 4), 16) % 360;
  const h2 = (h1 + 40 + (parseInt(address.slice(4, 6), 16) % 120)) % 360;
  const tile = el("span", { className: `avatar${small ? " sm" : ""}` });
  tile.style.background = `linear-gradient(135deg, hsl(${h1} 75% 58%), hsl(${h2} 70% 42%))`;
  return tile;
}

function hashView(hash) {
  const zeros = leadingZeros(hash);
  return el("div", { className: "hash", title: hash },
    el("span", { className: "zeros", textContent: hash.slice(0, zeros) }), hash.slice(zeros));
}

const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
function ago(timestamp) {
  const s = Math.round((new Date(timestamp) - Date.now()) / 1000);
  for (const [unit, size] of [["day", 86400], ["hour", 3600], ["minute", 60]]) {
    if (Math.abs(s) >= size) return rtf.format(Math.round(s / size), unit);
  }
  return Math.abs(s) < 5 ? "just now" : rtf.format(s, "second");
}

function toast(message, isError = false) {
  const note = el("div", { className: `toast${isError ? " error" : ""}`, textContent: message });
  $("toasts").append(note);
  setTimeout(() => note.remove(), 3800);
}

// ---- Rendering --------------------------------------------------------------

function party(address) {
  return el("span", { className: "party" }, avatar(address, true), el("span", { textContent: nameOf(address) }));
}

function txView(tx) {
  if (tx.from === "coinbase") {
    return el("li", { className: "tx coinbase" },
      el("span", { className: "tag", textContent: "reward" }), el("span", { className: "arrow", textContent: "→" }),
      party(tx.to), el("span", { className: "amt", textContent: `+${fmt(tx.amount)}` }));
  }
  return el("li", { className: "tx", title: `nonce ${tx.nonce}` },
    party(tx.from), el("span", { className: "arrow", textContent: "→" }), party(tx.to),
    el("span", { className: "amt", textContent: fmt(tx.amount) }));
}

function renderStats() {
  $("stat-height").textContent = state.blocks.length ? fmt(state.blocks.length - 1) : "–";
  $("stat-difficulty").textContent = state.blocks.length ? difficulty() : "–";
  $("stat-supply").textContent = fmt(supply());
  $("stat-pending").textContent = fmt(state.pending.length);
  const pill = $("validity");
  pill.className = `pill ${state.valid ? "ok" : "bad"}`;
  pill.lastChild.textContent = state.valid ? "chain valid" : "chain INVALID";
}

async function renderWallets() {
  const accounts = await Promise.all(Wallets.list.map((w) => api("GET", `/balances/${w.address}`)));
  $("wallet-count").textContent = Wallets.list.length || "";
  $("wallets").replaceChildren(
    ...(Wallets.list.length
      ? Wallets.list.map((w, i) => walletView(w, accounts[i]))
      : [el("div", { className: "empty", textContent: "No wallets yet. Make one, then mine to fund it." })]),
  );
  fillSelect($("send-from"), false);
  fillSelect($("send-to"), true);
  fillSelect($("miner"), false);
}

function walletView(wallet, { balance, next_nonce }) {
  const copy = el("button", { className: "icon", type: "button", textContent: "⧉", title: "Copy address" });
  copy.onclick = () => navigator.clipboard.writeText(wallet.address).then(() => toast("Address copied"));
  const forget = el("button", { className: "icon forget", type: "button", textContent: "✕", title: "Forget this wallet" });
  forget.onclick = () => {
    if (!confirm(`Forget ${wallet.name}? Its secret key will be gone from this browser.`)) return;
    Wallets.remove(wallet);
    refresh();
  };
  return el("div", { className: "wallet" },
    avatar(wallet.address),
    el("div", { className: "who" },
      el("div", { className: "name", textContent: wallet.name }),
      el("div", { className: "addr", title: wallet.address }, short(wallet.address), copy)),
    el("div", { className: "money" },
      el("div", { className: `balance${balance ? "" : " zero"}`, textContent: fmt(balance) }),
      el("div", { className: "nonce", textContent: `nonce ${next_nonce}` })),
    forget);
}

function fillSelect(select, includeOthers) {
  const current = select.value;
  select.replaceChildren(...Wallets.list.map((w) => el("option", { value: w.address, textContent: w.name })));
  if (includeOthers) select.append(el("option", { value: "__other", textContent: "Other address…" }));
  if ([...select.options].some((o) => o.value === current)) select.value = current;
}

function renderPending() {
  $("pending").replaceChildren(
    ...(state.pending.length
      ? state.pending.map(txView)
      : [el("li", { className: "empty", textContent: "Nothing waiting to be mined." })]),
  );
}

function renderChain() {
  const fresh = state.seenTip !== null && state.tip !== state.seenTip;
  state.seenTip = state.tip;
  $("blocks").replaceChildren(
    ...state.blocks.slice().reverse().map((b, i) => {
      const genesis = b.index === 0;
      return el("li", { className: `block${genesis ? " genesis" : ""}${fresh && i === 0 ? " fresh" : ""}` },
        el("div", { className: "marker", textContent: b.index }),
        el("article", { className: "card" },
          el("div", { className: "card-head" },
            el("strong", { textContent: genesis ? "Genesis" : `Block ${b.index}` }),
            genesis ? null : el("span", { className: "when", textContent: ago(b.timestamp), title: new Date(b.timestamp).toLocaleString() }),
            el("span", { className: "ntx", textContent: `${b.transactions.length} tx` })),
          hashView(b.hash),
          el("div", { className: "meta" },
            el("span", {}, "prev ", el("span", { className: "mono", textContent: short(b.prev_hash) })),
            el("span", {}, "nonce ", el("span", { className: "mono", textContent: fmt(b.nonce) }))),
          b.transactions.length ? el("ul", { className: "txs" }, ...b.transactions.map(txView)) : null));
    }),
  );
}

async function refresh() {
  try {
    await load();
    await renderWallets();
    renderStats();
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

$("wallet-form").onsubmit = (e) => {
  e.preventDefault();
  busy($("create-wallet"), async () => {
    const { address, secret_key } = await api("POST", "/wallets");
    const name = $("wallet-name").value.trim() || `Wallet ${Wallets.list.length + 1}`;
    Wallets.add({ name, address, secret_key });
    $("wallet-name").value = "";
    toast(`Created ${name}`);
  });
};

$("send-form").onsubmit = (e) => {
  e.preventDefault();
  busy($("send"), async () => {
    const from = Wallets.byAddress($("send-from").value);
    if (!from) throw new Error("Create a wallet first");
    let to = $("send-to").value;
    if (to === "__other") to = (prompt("Recipient address (hex)") || "").trim();
    if (!to) return;
    const amount = Number($("send-amount").value);
    const tx = await api("POST", "/transactions/sign", { secret_key: from.secret_key, to, amount });
    await api("POST", "/transactions", tx);
    toast(`Queued ${fmt(amount)} from ${from.name}. Mine a block to confirm it.`);
  });
};

$("mine-form").onsubmit = (e) => {
  e.preventDefault();
  const miner = $("miner").value;
  if (!miner) return toast("Create a wallet first", true);
  const status = $("mining");
  const started = performance.now();
  const zeros = "0".repeat(difficulty());
  const tick = () => {
    const secs = ((performance.now() - started) / 1000).toFixed(1);
    status.replaceChildren(el("span", { className: "spin" }),
      el("span", {}, "Looking for a hash starting with ", el("span", { className: "mono", textContent: zeros }), ` · ${secs}s`));
  };
  tick();
  status.hidden = false;
  const timer = setInterval(tick, 100);
  busy($("mine"), async () => {
    try {
      const block = await api("POST", "/mine", { miner });
      const secs = ((performance.now() - started) / 1000).toFixed(2);
      status.replaceChildren(el("span", {}, `Block ${block.index} found after `,
        el("span", { className: "mono", textContent: fmt(block.nonce + 1) }), ` tries in ${secs}s`));
      setTimeout(() => (status.hidden = true), 5000);
    } catch (err) {
      status.hidden = true;
      throw err;
    } finally {
      clearInterval(timer);
    }
  });
};

// Pick up changes made elsewhere, such as another node's chain offered with POST /chain.
setInterval(async () => {
  if (document.hidden || working) return;
  try {
    const [blocks, pending] = await Promise.all([api("GET", "/chain"), api("GET", "/transactions/pending")]);
    if (blocks.at(-1)?.hash !== state.tip || pending.length !== state.pending.length) refresh();
  } catch { /* node down; the next tick tries again */ }
}, POLL_MS);

refresh();
