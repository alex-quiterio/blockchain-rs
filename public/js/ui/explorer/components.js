// Small views the explorer's pages share: links, verdicts that arrive once a check finishes, fact
// lists, panels and tables of transactions.

import { COINBASE, STAKE, UNSTAKE, kindOf } from "../../domain/transfers.js";
import { transferIsSigned } from "../../domain/verification/signatures.js";
import { el, fmt, short } from "../dom.js";

export const href = { block: (id) => `#/block/${id}`, tx: (sig) => `#/tx/${sig}`, address: (a) => `#/address/${a}` };
export const link = (to, text, className = "mono") => el("a", { href: to, className, textContent: text });

/** An address as a link, or the special names it can take. */
export function who(address) {
  if (address === COINBASE) return el("span", { className: "tag", textContent: "new coins" });
  if (address === STAKE || address === UNSTAKE) return el("span", { className: `tag ${address}`, textContent: address });
  return link(href.address(address), short(address));
}

const ok = (text) => el("span", { className: "ok", textContent: `✓ ${text}` });
const bad = (text) => el("span", { className: "bad", textContent: `✗ ${text}` });

/** A row of the facts list whose verdict arrives later: `check` resolves to [passed, text]. */
export function checkRow(facts, label, check, ...detail) {
  const verdict = el("span", { className: "muted", textContent: "checking…" });
  check.then(([passed, text]) => verdict.replaceWith(passed ? ok(text) : bad(text)))
    .catch((e) => verdict.replaceWith(bad(e.message)));
  facts.append(el("dt", { textContent: label }), el("dd", {}, verdict, detail.length ? " " : null, detail));
}

export function factRow(facts, label, ...value) {
  facts.append(el("dt", { textContent: label }), el("dd", {}, value));
}

export const panel = (title, ...children) => el("section", { className: "panel" }, el("h2", {}, title), children);
export const formula = (text) => el("span", { className: "formula", textContent: text });

export function table(headings, rows) {
  return el("div", { className: "grid-wrap" },
    el("table", { className: "grid" },
      el("thead", {}, el("tr", {}, headings.map((h) => el("th", { textContent: h })))),
      el("tbody", {}, rows)));
}

/** Rows of transactions, each signature re-checked here. `perspective` turns amounts into + and − for one address. */
export function txTable(entries, inBlock = false, perspective = null) {
  const rows = entries.map(({ transaction: tx, status, block }) => {
    const kind = kindOf(tx);
    const sign = perspective ? (tx.from === perspective ? "−" : "+") : "";
    const verdict = el("span", { className: "muted", textContent: "…" });
    (kind === "coinbase" ? Promise.resolve(null) : transferIsSigned(tx))
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
