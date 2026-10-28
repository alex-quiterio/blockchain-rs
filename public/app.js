const $ = (id) => document.getElementById(id);
const POLL_MS = 4000;
const COINBASE = "coinbase";
const STAKE = "stake";
const UNSTAKE = "unstake";

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
  list: [],
  load() {
    try { this.list = JSON.parse(localStorage.getItem(this.KEY) || "[]"); } catch { this.list = []; }
  },
  save() {
    try { localStorage.setItem(this.KEY, JSON.stringify(this.list)); } catch { /* private mode */ }
  },
  add(wallet) { this.list.push(wallet); this.save(); },
  remove(wallet) { this.list = this.list.filter((w) => w !== wallet); this.save(); },
  byAddress(address) { return this.list.find((w) => w.address === address); },
};
Wallets.load();

// ---- Chain state, as last seen ---------------------------------------------

const state = { blocks: [], pending: [], valid: null, consensus: null, accounts: {}, peers: [], node: null, seenTip: null };

const isStake = () => state.consensus?.kind === "proof_of_stake";
const tip = () => state.blocks.at(-1)?.hash;
const kindOf = (tx) => (tx.from === COINBASE ? "coinbase" : tx.to === STAKE ? "stake" : tx.to === UNSTAKE ? "unstake" : "transfer");
const feesOf = (txs) => txs.reduce((sum, tx) => sum + (tx.fee || 0), 0);
// Fees only move coins around; the supply grows by the block reward alone.
const supply = () => (state.blocks.length - 1) * (state.consensus?.reward ?? 0);

async function load() {
  const [blocks, pending, validity, consensus, peers, node] = await Promise.all([
    api("GET", "/chain"), api("GET", "/transactions/pending"), api("GET", "/chain/valid"),
    api("GET", "/consensus"), api("GET", "/peers"), state.node || api("GET", "/node"),
  ]);
  const accounts = await Promise.all(Wallets.list.map((w) => api("GET", `/balances/${w.address}`)));
  Object.assign(state, {
    blocks, pending, consensus, peers, node, valid: validity.valid,
    accounts: Object.fromEntries(accounts.map((a) => [a.address, a])),
  });
}

// ---- View helpers -----------------------------------------------------------

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  Object.assign(node, attrs);
  node.append(...children.flat().filter((c) => c != null && c !== false));
  return node;
}

const short = (hex) => (hex.length > 16 ? `${hex.slice(0, 8)}…${hex.slice(-6)}` : hex);
const nameOf = (address) => Wallets.byAddress(address)?.name || short(address);
const fmt = (n) => Number(n).toLocaleString();
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

function hashView(hash) {
  const zeros = hash.match(/^0*/)[0].length;
  const shown = isStake() ? 0 : Math.min(zeros, state.consensus?.difficulty ?? zeros);
  return el("div", { className: "hash", title: hash },
    el("span", { className: "zeros", textContent: hash.slice(0, shown) }), hash.slice(shown));
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
      hashView(block.hash),
      el("div", { className: "meta" },
        forged
          ? el("span", {}, "forged by ", party(block.validator))
          : el("span", {}, "nonce ", el("span", { className: "mono", textContent: fmt(block.nonce) })),
        el("span", {}, "prev ", el("span", { className: "mono", textContent: short(block.prev_hash) }))),
      block.transactions.length ? el("ul", { className: "txs" }, block.transactions.map(txView)) : null,
      breakdown(block, prev, payout, rest)));
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
      ". The validator was drawn at random, weighted by stake, using the previous hash as the seed, so every node agrees whose turn it was.");
  } else {
    const d = state.consensus?.difficulty ?? block.hash.match(/^0*/)[0].length;
    row("Proof", yes(`starts with ${"0".repeat(d)}`),
      ` Found on try ${fmt(block.nonce + 1)}. Each try changes the nonce and rehashes; on average it takes ${fmt(16 ** d)} tries.`);
  }
  if (payout) {
    const fees = feesOf(transfers);
    row("Payout", `${fmt(payout.amount - fees)} reward + ${fmt(fees)} in fees = `, el("b", { textContent: fmt(payout.amount) }), " to ", party(payout.to));
  }
  const fields = ["index", "timestamp", "prev_hash", "transactions", "nonce", block.validator && "validator"].filter(Boolean);
  row("Hash", el("span", { className: "formula", textContent: `sha256(${fields.join(" | ")})` }));
  return el("details", { className: "breakdown" }, el("summary", { textContent: "Breakdown" }), el("dl", { className: "facts" }, facts));
}

// ---- Panels -----------------------------------------------------------------

function renderStats() {
  const c = state.consensus;
  $("stat-consensus").textContent = c ? (isStake() ? "Proof of stake" : `Proof of work · ${c.difficulty}`) : "–";
  $("stat-height").textContent = state.blocks.length ? fmt(state.blocks.length - 1) : "–";
  $("stat-supply").textContent = fmt(supply());
  $("stat-pending").textContent = fmt(state.pending.length);
  const pill = $("validity");
  pill.className = `pill ${state.valid ? "ok" : "bad"}`;
  pill.lastChild.textContent = state.valid ? "chain valid" : "chain INVALID";
}

function renderWallets() {
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
  const d = state.consensus?.difficulty ?? 0;
  $("producer").replaceChildren(form, explain("How does proof of work work?",
    `The miner bundles the mempool and a payout to themselves into a block, then hashes it again and again, changing the <code>nonce</code> each time.`,
    `The block counts only once its hash starts with <code>${"0".repeat(d)}</code>. That takes about ${fmt(16 ** d)} tries, so rewriting history means redoing all that work.`,
    `When forks disagree, the chain with the most total work wins. Exact ties go to the lower tip hash, so every node picks the same one.`));
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
    `For each block, one validator is drawn at random, weighted by stake. The previous block's hash seeds the draw, so every node picks the same one.`,
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
  $("blocks").replaceChildren(
    ...state.blocks.slice().reverse().map((block, i) => {
      const view = blockView(block, state.blocks[block.index - 1], fresh && i === 0);
      const details = view.querySelector(".breakdown");
      details.dataset.hash = block.hash;
      details.open = open.has(block.hash);
      return view;
    }),
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
  const ours = state.blocks.length;
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
  const done = showProducing(`Looking for a hash starting with ${"0".repeat(state.consensus.difficulty)}`);
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

function forge(button, wallet) {
  if (!wallet) return toast("Create a wallet first", true);
  busy(button, async () => {
    const block = await api("POST", "/forge", { secret_key: wallet.secret_key });
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
    const { address, secret_key } = await api("POST", "/wallets");
    const name = $("wallet-name").value.trim() || `Wallet ${Wallets.list.length + 1}`;
    Wallets.add({ name, address, secret_key });
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
    let to = $("send-to").value;
    if (to === "__stake") to = STAKE;
    else if (to === "__unstake") to = UNSTAKE;
    else if (to === "__other") to = (prompt("Recipient address (hex)") || "").trim();
    if (!to) return;
    const amount = Number($("send-amount").value);
    const fee = Number($("send-fee").value) || 0;
    const tx = await api("POST", "/transactions/sign", { secret_key: from.secret_key, to, amount, fee });
    await api("POST", "/transactions", tx);
    toast(`Queued from ${from.name}. It confirms in the next block.`);
  });
};

// Pick up changes made elsewhere: other tabs, other nodes syncing, `POST /chain`.
setInterval(async () => {
  if (document.hidden || working) return;
  try {
    const [blocks, pending, peers] = await Promise.all([
      api("GET", "/chain"), api("GET", "/transactions/pending"), api("GET", "/peers"),
    ]);
    if (blocks.at(-1)?.hash !== tip() || pending.length !== state.pending.length) return refresh();
    state.peers = peers;
    renderPeers();
  } catch { /* node down; the next tick tries again */ }
}, POLL_MS);

refresh();
