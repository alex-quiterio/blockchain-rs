const $ = (id) => document.getElementById(id);
const STORE = "blockchain-rs:wallets";

let wallets = JSON.parse(localStorage.getItem(STORE) || "[]");
const saveWallets = () => localStorage.setItem(STORE, JSON.stringify(wallets));

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

function toast(message, isError = false) {
  const el = $("toast");
  el.textContent = message;
  el.className = isError ? "error" : "";
  el.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => (el.hidden = true), 3500);
}

const short = (hex) => (hex.length > 16 ? `${hex.slice(0, 8)}…${hex.slice(-6)}` : hex);
const nameOf = (address) => wallets.find((w) => w.address === address)?.name || short(address);

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  Object.assign(node, attrs);
  node.append(...children);
  return node;
}

function fillSelect(select, includeOthers) {
  const current = select.value;
  select.replaceChildren(...wallets.map((w) => el("option", { value: w.address, textContent: w.name })));
  if (includeOthers) select.append(el("option", { value: "__other", textContent: "Other address…" }));
  if ([...select.options].some((o) => o.value === current)) select.value = current;
}

async function renderWallets() {
  const rows = await Promise.all(
    wallets.map(async (w) => {
      const { balance, next_nonce } = await api("GET", `/balances/${w.address}`);
      const copy = el("button", { className: "link", textContent: "copy", title: "Copy address" });
      copy.onclick = () => navigator.clipboard.writeText(w.address).then(() => toast("Address copied"));
      const forget = el("button", { className: "link", textContent: "×", title: "Forget this wallet" });
      forget.onclick = () => {
        if (!confirm(`Forget ${w.name}? Its secret key will be gone from this browser.`)) return;
        wallets = wallets.filter((x) => x !== w);
        saveWallets();
        refresh();
      };
      return el("tr", {},
        el("td", { textContent: w.name }),
        el("td", { className: "mono", textContent: short(w.address), title: w.address }),
        el("td", { textContent: balance }),
        el("td", { textContent: next_nonce }),
        el("td", {}, copy, forget));
    }),
  );
  $("wallets").replaceChildren(...rows);
  fillSelect($("send-from"), false);
  fillSelect($("send-to"), true);
  fillSelect($("miner"), false);
}

function txLine(tx) {
  const text = tx.from === "coinbase"
    ? `⛏ reward ${tx.amount} → ${nameOf(tx.to)}`
    : `${nameOf(tx.from)} → ${nameOf(tx.to)}: ${tx.amount} (nonce ${tx.nonce})`;
  return el("li", { className: tx.from === "coinbase" ? "coinbase" : "", textContent: text });
}

async function renderPending() {
  const pending = await api("GET", "/transactions/pending");
  $("pending").replaceChildren(...(pending.length ? pending.map(txLine) : [el("li", { className: "hint", textContent: "Nothing waiting." })]));
}

async function renderChain() {
  const [blocks, validity] = await Promise.all([api("GET", "/chain"), api("GET", "/chain/valid")]);
  $("blocks").replaceChildren(
    ...blocks.map((b) =>
      el("div", { className: "block" },
        el("div", { className: "block-head" },
          el("strong", { textContent: b.index === 0 ? "Genesis" : `Block ${b.index}` }),
          el("span", { className: "hint", textContent: new Date(b.timestamp).toLocaleString() })),
        el("div", { className: "mono", textContent: `hash ${b.hash}` }),
        el("div", { className: "mono hint", textContent: `prev ${short(b.prev_hash)} · nonce ${b.nonce}` }),
        el("ul", { className: "txs" }, ...b.transactions.map(txLine)))),
  );
  const badge = $("validity");
  badge.textContent = validity.valid ? `valid · ${validity.length} blocks` : "INVALID";
  badge.className = `badge ${validity.valid ? "ok" : "bad"}`;
}

async function refresh() {
  try {
    await Promise.all([renderWallets(), renderPending(), renderChain()]);
  } catch (err) {
    toast(err.message, true);
  }
}

async function busy(button, action) {
  button.disabled = true;
  try {
    await action();
    await refresh();
  } catch (err) {
    toast(err.message, true);
  } finally {
    button.disabled = false;
  }
}

$("create-wallet").onclick = (e) =>
  busy(e.target, async () => {
    const { address, secret_key } = await api("POST", "/wallets");
    const name = $("wallet-name").value.trim() || `Wallet ${wallets.length + 1}`;
    wallets.push({ name, address, secret_key });
    saveWallets();
    $("wallet-name").value = "";
    toast(`Created ${name}`);
  });

$("send").onclick = (e) =>
  busy(e.target, async () => {
    const from = wallets.find((w) => w.address === $("send-from").value);
    if (!from) throw new Error("Create a wallet first");
    let to = $("send-to").value;
    if (to === "__other") to = (prompt("Recipient address (hex)") || "").trim();
    if (!to) return;
    const amount = Number($("send-amount").value);
    const tx = await api("POST", "/transactions/sign", { secret_key: from.secret_key, to, amount });
    await api("POST", "/transactions", tx);
    toast(`Queued ${amount} from ${from.name}. Mine a block to confirm it.`);
  });

$("mine").onclick = (e) =>
  busy(e.target, async () => {
    const miner = $("miner").value;
    if (!miner) throw new Error("Create a wallet first");
    const block = await api("POST", "/mine", { miner });
    toast(`Mined block ${block.index} (nonce ${block.nonce})`);
  });

refresh();
