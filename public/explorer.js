// The block explorer: every block, transaction and address the node knows, with each of the
// node's checks redone in this page. Routes live in the URL hash, so any view can be linked to.

const POLL_MS = 4000;
const LATEST = 20;

const ctx = { consensus: null, works: [], tip: null };

// ---- Links and small views --------------------------------------------------

const href = { block: (id) => `#/block/${id}`, tx: (sig) => `#/tx/${sig}`, address: (a) => `#/address/${a}` };
const link = (to, text, className = "mono") => el("a", { href: to, className, textContent: text });

/** An address as a link, or the special names it can take. */
function who(address) {
  if (address === COINBASE) return el("span", { className: "tag", textContent: "new coins" });
  if (address === STAKE || address === UNSTAKE) return el("span", { className: `tag ${address}`, textContent: address });
  return link(href.address(address), short(address));
}

const ok = (text) => el("span", { className: "ok", textContent: `✓ ${text}` });
const bad = (text) => el("span", { className: "bad", textContent: `✗ ${text}` });

/** A row of the facts list whose verdict arrives later: `check` resolves to [passed, text]. */
function checkRow(facts, label, check, ...detail) {
  const verdict = el("span", { className: "muted", textContent: "checking…" });
  check.then(([passed, text]) => verdict.replaceWith(passed ? ok(text) : bad(text)))
    .catch((e) => verdict.replaceWith(bad(e.message)));
  facts.append(el("dt", { textContent: label }), el("dd", {}, verdict, detail.length ? " " : null, detail));
}

function factRow(facts, label, ...value) {
  facts.append(el("dt", { textContent: label }), el("dd", {}, value));
}

const panel = (title, ...children) => el("section", { className: "panel" }, el("h2", {}, title), children);
const formula = (text) => el("span", { className: "formula", textContent: text });

function table(headings, rows) {
  return el("div", { className: "grid-wrap" },
    el("table", { className: "grid" },
      el("thead", {}, el("tr", {}, headings.map((h) => el("th", { textContent: h })))),
      el("tbody", {}, rows)));
}

function toast(message) {
  const note = el("div", { className: "toast error", textContent: message });
  $("toasts").append(note);
  setTimeout(() => note.remove(), 5000);
}

// ---- Views ------------------------------------------------------------------

async function home() {
  const status = await api("GET", "/status");
  const from = Math.max(0, status.height - LATEST + 1);
  const [blocks, pending] = await Promise.all([api("GET", `/chain?from=${from}`), api("GET", "/transactions/pending")]);

  const facts = el("dl", { className: "facts" });
  factRow(facts, "Consensus", ctx.consensus.kind === "proof_of_stake" ? "proof of stake" : "proof of work");
  factRow(facts, "Height", link(href.block(status.height), fmt(status.height)));
  factRow(facts, "Tip", link(href.block(status.tip), short(status.tip)));
  factRow(facts, "Weight", fmt(status.weight), ctx.consensus.kind === "proof_of_stake" ? " stake behind the blocks" : " expected hashes behind the blocks");
  if (ctx.consensus.work) factRow(facts, "Next block", `needs ${fmt(ctx.consensus.work)} hashes on average`);

  const rows = blocks.slice().reverse().map((block) => {
    const [payout, ...transfers] = block.transactions;
    return el("tr", {},
      el("td", {}, link(href.block(block.index), `#${block.index}`)),
      el("td", {}, link(href.block(block.hash), short(block.hash))),
      el("td", { className: "muted", textContent: block.index ? ago(block.timestamp) : "the beginning" }),
      el("td", { className: "num", textContent: fmt(transfers.length) }),
      el("td", { className: "num", textContent: fmt(feesOf(transfers)) }),
      el("td", {}, payout ? who(payout.to) : "–"));
  });

  return [
    panel("Network", facts),
    panel(`Latest blocks`, table(["Height", "Hash", "When", "Transfers", "Fees", "Paid to"], rows)),
    panel(`Mempool (${pending.length})`, pending.length
      ? txTable(pending.map((tx) => ({ transaction: tx, status: "pending" })))
      : el("p", { className: "hint", textContent: "Nothing waiting. Transfers sit here until a block takes them." })),
  ];
}

async function blockPage(id) {
  const block = await api("GET", `/blocks/${encodeURIComponent(id)}`);
  const prev = block.index ? await api("GET", `/blocks/${block.index - 1}`) : null;
  const [payout, ...transfers] = block.transactions;
  const isTip = block.hash === ctx.tip;

  const facts = el("dl", { className: "facts" });
  factRow(facts, "Hash", el("span", { className: "mono wrap", textContent: block.hash }));
  factRow(facts, "Time", new Date(block.timestamp).toLocaleString(), block.index ? ` · ${ago(block.timestamp)}` : "");
  checkRow(facts, "Header hash", Check.headerHash(block).then((h) => [h === block.hash, "recomputed here and matches"]),
    formula("sha256(index | timestamp | prev_hash | merkle_root | nonce" + (block.validator ? " | validator | reveal" : "") + ")"));
  if (prev) {
    checkRow(facts, "Link", Promise.resolve([prev.hash === block.prev_hash, `prev_hash is block #${prev.index}'s hash`]),
      link(href.block(prev.index), short(block.prev_hash)));
  } else {
    factRow(facts, "Link", "Genesis: the fixed first block every node builds the same way.");
  }
  if (block.validator) {
    checkRow(facts, "Proof", Check.signedBy(block.validator, block.hash, block.signature).then((v) => [v, "the validator's signature over the hash"]),
      "by ", who(block.validator));
  } else {
    const work = ctx.works[block.index];
    checkRow(facts, "Proof of work", Promise.resolve([work != null && Check.meetsWork(block.hash, work), `hash is below the target for ${fmt(work)} hashes`]),
      `nonce ${fmt(block.nonce)}`);
  }
  checkRow(facts, "Merkle root", Merkle.root(block.transactions).then((r) => [r === block.merkle_root, `recomputed from the ${block.transactions.length} transactions`]),
    el("span", { className: "mono", textContent: short(block.merkle_root) }));
  if (transfers.length) {
    checkRow(facts, "Signatures", Promise.all(transfers.map((tx) => Check.signedBy(tx.from, Check.signingPayload(tx), tx.signature)))
      .then((results) => [results.every(Boolean), `${results.filter(Boolean).length} of ${results.length} verified here`]));
  }
  if (payout) {
    const fees = feesOf(transfers);
    checkRow(facts, "Payout", Promise.resolve([payout.amount === ctx.consensus.reward + fees, `${fmt(ctx.consensus.reward)} reward + ${fmt(fees)} in fees = ${fmt(payout.amount)}`]),
      "to ", who(payout.to));
  }
  factRow(facts, "Confirmations", fmt(ctx.height - block.index + 1), isTip ? " (the tip)" : "");

  const nav = el("div", { className: "pager" },
    block.index ? link(href.block(block.index - 1), `← #${block.index - 1}`, "") : el("span"),
    isTip ? el("span") : link(href.block(block.index + 1), `#${block.index + 1} →`, ""));

  const rows = block.transactions.map((tx, position) => ({ transaction: tx, status: "confirmed", block: { height: block.index, position } }));
  return [
    el("h1", {}, block.index ? `Block #${block.index}` : "Genesis", " ", el("span", { className: `badge-kind ${block.validator ? "pos" : "pow"}`, textContent: block.validator ? "forged" : "mined" })),
    nav,
    panel("Why it's valid", facts),
    panel(`Transactions (${block.transactions.length})`, rows.length ? txTable(rows, true) : el("p", { className: "hint", textContent: "None: genesis carries no transactions." })),
  ];
}

async function txPage(signature) {
  const found = await api("GET", `/transactions/${encodeURIComponent(signature)}`);
  const tx = found.transaction;
  const placed = found.block;
  const block = placed ? await api("GET", `/blocks/${placed.height}`) : null;
  const kind = kindOf(tx);

  const summary = el("dl", { className: "facts" });
  factRow(summary, "Status", placed
    ? el("span", { className: "ok", textContent: `confirmed, ${fmt(placed.confirmations)} confirmation${placed.confirmations === 1 ? "" : "s"}` })
    : el("span", { className: "pending", textContent: "pending: waiting in the mempool for a block" }));
  if (placed) {
    factRow(summary, "Block", link(href.block(placed.height), `#${placed.height}`), ` · position ${placed.position} of ${block.transactions.length} · ${ago(placed.timestamp)}`);
  }
  factRow(summary, "Kind", el("span", { className: `tag ${kind}`, textContent: kind }));
  factRow(summary, "From", who(tx.from));
  factRow(summary, "To", who(tx.to));
  factRow(summary, "Amount", el("b", { textContent: fmt(tx.amount) }), tx.fee ? ` + ${fmt(tx.fee)} fee` : " · no fee");
  factRow(summary, "Nonce", fmt(tx.nonce), ` · the sender's transaction number ${fmt(tx.nonce + 1)}. Each nonce is used once, which stops replays.`);
  if (tx.commitment) factRow(summary, "Commitment", el("span", { className: "mono wrap", textContent: tx.commitment }), " · the end of the staker's reveal chain");

  const signed = el("dl", { className: "facts" });
  const payload = Check.signingPayload(tx);
  factRow(signed, "Payload", el("span", { className: "formula wrap", textContent: payload }));
  factRow(signed, "Signature", el("span", { className: "mono wrap", textContent: tx.signature }));
  checkRow(signed, "Verified", Check.signedBy(tx.from, payload, tx.signature).then((v) => [v, "ed25519, against the sender's address as public key"]),
    "Change any field and this fails, which is why nobody can alter a transfer on its way.");

  const effects = el("dl", { className: "facts" });
  const producer = block?.transactions[0]?.to;
  if (kind === "stake") {
    factRow(effects, "Sender", `−${fmt(tx.amount + (tx.fee || 0))} spendable, +${fmt(tx.amount)} staked`);
  } else if (kind === "unstake") {
    factRow(effects, "Sender", `+${fmt(tx.amount)} spendable, −${fmt(tx.amount)} staked${tx.fee ? `, −${fmt(tx.fee)} fee` : ""}`);
  } else {
    factRow(effects, "Sender", `−${fmt(tx.amount + (tx.fee || 0))}`, " ", who(tx.from));
    factRow(effects, "Recipient", `+${fmt(tx.amount)}`, " ", who(tx.to));
  }
  if (tx.fee) factRow(effects, "Fee", `+${fmt(tx.fee)} to `, producer ? who(producer) : "whoever adds the block");

  const views = [
    el("h1", {}, "Transaction ", el("span", { className: "mono muted", textContent: short(tx.signature) })),
    panel("What it is", summary),
    panel("Who signed it", signed),
    panel("What it changes", effects),
  ];
  if (placed) views.push(panel("Proof it's in the block", ...(await proofView(tx, placed))));
  return views;
}

/** The Merkle path from this transaction up to its block's root, each hash redone here. */
async function proofView(tx, placed) {
  const proof = await api("GET", `/transactions/${encodeURIComponent(tx.signature)}/proof`);
  const leaf = await Merkle.leaf(tx);
  const path = await Check.climb(leaf, proof.proof);
  const header = proof.header;

  const steps = el("ol", { className: "path" },
    el("li", {}, el("span", { className: "mono", textContent: short(leaf) }), " leaf: ", formula("sha256(leaf | the transaction's JSON)")),
    proof.proof.map((step, i) => el("li", {},
      el("span", { className: "mono", textContent: short(path[i + 1]) }), " ",
      formula(step.side === "left" ? `sha256(node | ${short(step.hash)} | ↑)` : `sha256(node | ↑ | ${short(step.hash)})`),
      el("span", { className: "muted", textContent: ` sibling on the ${step.side}` }))));

  const facts = el("dl", { className: "facts" });
  checkRow(facts, "Leaf", Promise.resolve([leaf === proof.leaf, "hashed here and matches the node's"]));
  checkRow(facts, "Root", Promise.resolve([path.at(-1) === header.merkle_root, `${proof.proof.length} hashes lead to the block's Merkle root`]),
    el("span", { className: "mono", textContent: short(header.merkle_root) }));
  checkRow(facts, "Header", Check.headerHash(header).then((h) => [h === header.hash && h === placed.hash, `its hash covers that root and is block #${header.index}'s`]));
  factRow(facts, "Why it matters", `A light client keeps only headers (${fmt(header.index + 1)} so far) and checks this proof instead of downloading every transaction in the block.`);
  return [facts, steps];
}

async function addressPage(address) {
  const [account, history] = await Promise.all([
    api("GET", `/balances/${encodeURIComponent(address)}`),
    api("GET", `/addresses/${encodeURIComponent(address)}/transactions`),
  ]);
  const confirmed = history.filter((h) => h.block);
  const sum = (pick) => confirmed.reduce((total, h) => total + pick(h.transaction), 0);
  const mine = (tx) => tx.from === address;
  const rewards = sum((tx) => (tx.to === address && tx.from === COINBASE ? tx.amount : 0));
  const received = sum((tx) => (tx.to === address && tx.from !== COINBASE ? tx.amount : 0));
  const sent = sum((tx) => (mine(tx) && kindOf(tx) === "transfer" ? tx.amount : 0));
  const fees = sum((tx) => (mine(tx) ? tx.fee || 0 : 0));
  const staked = sum((tx) => (mine(tx) && tx.to === STAKE ? tx.amount : 0)) - sum((tx) => (mine(tx) && tx.to === UNSTAKE ? tx.amount : 0));

  const facts = el("dl", { className: "facts" });
  factRow(facts, "Address", el("span", { className: "mono wrap", textContent: address }), " · also its ed25519 public key");
  factRow(facts, "Balance", el("b", { textContent: fmt(account.balance) }), account.staked ? ` · ${fmt(account.staked)} staked` : "");
  factRow(facts, "In", `${fmt(rewards)} from block rewards, ${fmt(received)} from transfers`);
  factRow(facts, "Out", `${fmt(sent)} sent, ${fmt(fees)} in fees`, staked ? `, ${fmt(staked)} locked as stake` : "");
  checkRow(facts, "Adds up", Promise.resolve([
    rewards + received - sent - fees - staked === account.balance && staked === account.staked,
    "in − out − stake = balance, from its history alone"]));
  factRow(facts, "Next nonce", fmt(account.next_nonce), " · its next transfer must carry this number");

  return [
    el("h1", {}, "Address ", el("span", { className: "mono muted", textContent: short(address) })),
    panel("Account", facts),
    panel(`History (${history.length})`, history.length
      ? txTable(history, false, address)
      : el("p", { className: "hint", textContent: "Nothing yet. An address appears on the chain only once coins move to or from it." })),
  ];
}

/** Rows of transactions. `perspective` turns amounts into + and − for one address. */
function txTable(entries, inBlock = false, perspective = null) {
  const rows = entries.map(({ transaction: tx, status, block }) => {
    const kind = kindOf(tx);
    const sign = perspective ? (tx.from === perspective ? "−" : "+") : "";
    const verdict = el("span", { className: "muted", textContent: "…" });
    (kind === "coinbase" ? Promise.resolve(null) : Check.signedBy(tx.from, Check.signingPayload(tx), tx.signature))
      .then((v) => verdict.replaceWith(v === null ? el("span", { className: "muted", textContent: "–" }) : v ? ok("") : bad("")));
    return el("tr", {},
      inBlock ? el("td", { className: "num muted", textContent: block.position }) : el("td", {},
        status === "pending" ? el("span", { className: "pending", textContent: "pending" }) : link(href.block(block.height), `#${block.height}`)),
      el("td", {}, el("span", { className: `tag ${kind}`, textContent: kind })),
      el("td", {}, who(tx.from)),
      el("td", {}, who(tx.to)),
      el("td", { className: "num", textContent: `${sign}${fmt(tx.amount)}` }),
      el("td", { className: "num", textContent: tx.fee ? fmt(tx.fee) : "" }),
      el("td", {}, verdict),
      el("td", {}, kind === "coinbase" ? el("span", { className: "muted", textContent: "no signature" }) : link(href.tx(tx.signature), "debug →", "")));
  });
  return table([inBlock ? "#" : "Block", "Kind", "From", "To", "Amount", "Fee", "Sig", ""], rows);
}

// ---- Routing ----------------------------------------------------------------

const HEX = /^[0-9a-f]+$/i;

/** Works out what was searched for. Addresses and block hashes are both 64 hex characters. */
async function search(query) {
  const q = query.trim();
  if (/^\d+$/.test(q)) return href.block(q);
  if (HEX.test(q) && q.length === 128) return href.tx(q);
  if (HEX.test(q) && q.length === 64) {
    return api("GET", `/blocks/${q}`).then(() => href.block(q), () => href.address(q));
  }
  throw new Error("Try a block height, a 64-character block hash or address, or a 128-character signature.");
}

async function render() {
  const [, kind, id] = location.hash.match(/^#\/(block|tx|address)\/(.+)$/) || [];
  const view = $("view");
  try {
    const parts = await ({ block: blockPage, tx: txPage, address: addressPage }[kind] || home)(id && decodeURIComponent(id));
    view.replaceChildren(...parts);
  } catch (e) {
    view.replaceChildren(panel("Not found", el("p", { className: "hint", textContent: e.message }), el("p", {}, link("#/", "← Back to the latest blocks", ""))));
  }
}

async function refreshContext() {
  const [status, consensus, works] = await Promise.all([api("GET", "/status"), api("GET", "/consensus"), api("GET", "/chain/work").catch(() => [])]);
  const changed = status.tip !== ctx.tip;
  Object.assign(ctx, { consensus, works: works.map(Number), tip: status.tip, height: status.height });
  return changed;
}

$("search").addEventListener("submit", async (event) => {
  event.preventDefault();
  try {
    location.hash = await search($("query").value);
  } catch (e) {
    toast(e.message);
  }
});
window.addEventListener("hashchange", () => { render(); window.scrollTo(0, 0); });

(async () => {
  await refreshContext();
  await render();
  // New blocks change confirmations, the mempool and the latest blocks, so redraw on each one.
  setInterval(async () => {
    if (await refreshContext().catch(() => false)) render();
  }, POLL_MS);
})();
