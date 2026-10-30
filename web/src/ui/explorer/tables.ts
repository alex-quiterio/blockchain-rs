// Tables on the explorer's pages, and the one for transactions, each signature re-checked here.

import { signatureOf } from "@application/audit";
import type { Located } from "@application/ports";
import { kindOf, type Transfer } from "@domain/transfers";
import { el, fmt } from "@ui/dom";
import { bad, ok } from "./facts.ts";
import { href, link, who } from "./links.ts";

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
