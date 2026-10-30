// Small views the explorer's pages share: links, verdicts that arrive once a check finishes, fact
// lists, panels and tables of transactions.

import { signatureOf } from "@application/audit";
import type { Located } from "@application/ports";
import {
  COINBASE,
  STAKE,
  UNSTAKE,
  kindOf,
  type Transfer,
} from "@domain/transfers";
import { el, fmt, short, type Child } from "@ui/dom";

export const href = {
  block: (id: string | number): string => `#/block/${id}`,
  tx: (sig: string): string => `#/tx/${sig}`,
  address: (a: string): string => `#/address/${a}`,
};
export const link = (
  to: string,
  text: string,
  className = "mono",
): HTMLAnchorElement => el("a", { href: to, className, textContent: text });

/** An address as a link, or the special names it can take. */
export function who(address: string): HTMLElement {
  if (address === COINBASE)
    return el("span", { className: "tag", textContent: "new coins" });
  if (address === STAKE || address === UNSTAKE)
    return el("span", { className: `tag ${address}`, textContent: address });
  return link(href.address(address), short(address));
}

const ok = (text: string): HTMLSpanElement =>
  el("span", { className: "ok", textContent: `✓ ${text}` });
const bad = (text: string): HTMLSpanElement =>
  el("span", { className: "bad", textContent: `✗ ${text}` });

/** A check's verdict: whether it passed, and what it says. */
export type Verdict = [passed: boolean, text: string];

/** A row of the facts list whose verdict arrives later. */
export function checkRow(
  facts: HTMLElement,
  label: string,
  check: Promise<Verdict>,
  ...detail: Child[]
): void {
  const verdict = el("span", { className: "muted", textContent: "checking…" });
  check
    .then(([passed, text]) =>
      verdict.replaceWith(passed ? ok(text) : bad(text)),
    )
    .catch((e: unknown) =>
      verdict.replaceWith(bad(e instanceof Error ? e.message : String(e))),
    );
  facts.append(
    el("dt", { textContent: label }),
    el("dd", {}, verdict, detail.length ? " " : null, detail),
  );
}

export function factRow(
  facts: HTMLElement,
  label: string,
  ...value: Child[]
): void {
  facts.append(el("dt", { textContent: label }), el("dd", {}, value));
}

export const panel = (title: string, ...children: Child[]): HTMLElement =>
  el("section", { className: "panel" }, el("h2", {}, title), children);
export const formula = (text: string): HTMLSpanElement =>
  el("span", { className: "formula", textContent: text });

export function table(
  headings: string[],
  rows: HTMLTableRowElement[],
): HTMLDivElement {
  return el(
    "div",
    { className: "grid-wrap" },
    el(
      "table",
      { className: "grid" },
      el(
        "thead",
        {},
        el(
          "tr",
          {},
          headings.map((h) => el("th", { textContent: h })),
        ),
      ),
      el("tbody", {}, rows),
    ),
  );
}

/** A transaction in a table: where it is, as a block and position, or still waiting. */
export interface TableEntry {
  transaction: Transfer;
  status: Located["status"];
  block?: { height: number; position: number } | null;
}

/**
 * Rows of transactions, each signature re-checked here. `perspective` turns amounts into + and −
 * for one address.
 */
export function txTable(
  entries: TableEntry[],
  inBlock = false,
  perspective: string | null = null,
): HTMLDivElement {
  const rows = entries.map(({ transaction: tx, status, block }) => {
    const kind = kindOf(tx);
    const sign = perspective ? (tx.from === perspective ? "−" : "+") : "";
    const verdict = el("span", { className: "muted", textContent: "…" });
    signatureOf(tx).then((v) =>
      verdict.replaceWith(
        v === null
          ? el("span", { className: "muted", textContent: "–" })
          : v
            ? ok("")
            : bad(""),
      ),
    );
    return el(
      "tr",
      {},
      inBlock
        ? el("td", {
            className: "num muted",
            textContent: String(block?.position ?? ""),
          })
        : el(
            "td",
            {},
            status === "pending" || !block
              ? el("span", { className: "pending", textContent: "pending" })
              : link(href.block(block.height), `#${block.height}`),
          ),
      el("td", {}, el("span", { className: `tag ${kind}`, textContent: kind })),
      el("td", {}, who(tx.from)),
      el("td", {}, who(tx.to)),
      el("td", { className: "num", textContent: `${sign}${fmt(tx.amount)}` }),
      el("td", { className: "num", textContent: tx.fee ? fmt(tx.fee) : "" }),
      el("td", {}, verdict),
      el(
        "td",
        {},
        kind === "coinbase"
          ? el("span", { className: "muted", textContent: "no signature" })
          : link(href.tx(tx.signature), "debug →", ""),
      ),
    );
  });
  return table(
    [inBlock ? "#" : "Block", "Kind", "From", "To", "Amount", "Fee", "Sig", ""],
    rows,
  );
}
