// A block's page: why it's valid, each of the node's checks redone here, and what it holds.

import { auditBlock } from "@application/audit";
import type { Lookup } from "@application/lookup";
import { ago, el, fmt, short } from "@ui/dom";
import {
  checkRow,
  factRow,
  formula,
  panel,
  type Verdict,
} from "@ui/explorer/facts";
import { href, link, who } from "@ui/explorer/links";
import { txTable, type TableEntry } from "@ui/explorer/tables";

export async function blockPage(
  lookup: Lookup,
  id: string,
): Promise<HTMLElement[]> {
  const audit = await auditBlock(lookup, id);
  const { context } = lookup;
  const { block } = audit;
  const isTip = block.hash === context.tip;

  const facts = el("dl", { className: "facts" });
  factRow(
    facts,
    "Hash",
    el("span", { className: "mono wrap", textContent: block.hash }),
  );
  factRow(
    facts,
    "Time",
    new Date(block.timestamp).toLocaleString(),
    block.index ? ` · ${ago(block.timestamp)}` : "",
  );
  checkRow(
    facts,
    "Header hash",
    audit.header.then((ok): Verdict => [ok, "recomputed here and matches"]),
    formula(
      "sha256(index | timestamp | prev_hash | merkle_root | nonce" +
        (block.validator ? " | validator | reveal" : "") +
        ")",
    ),
  );
  if (audit.link) {
    const { prev } = audit.link;
    checkRow(
      facts,
      "Link",
      Promise.resolve<Verdict>([
        audit.link.passed,
        `prev_hash is block #${prev.index}'s hash`,
      ]),
      link(href.block(prev.index), short(block.prev_hash)),
    );
  } else {
    factRow(
      facts,
      "Link",
      "Genesis: the fixed first block every node builds the same way.",
    );
  }
  const { seal } = audit;
  if (seal.by === "signature") {
    checkRow(
      facts,
      "Proof",
      seal.passed.then((ok): Verdict => [
        ok,
        "the validator's signature over the hash",
      ]),
      "by ",
      who(seal.validator),
    );
  } else {
    checkRow(
      facts,
      "Proof of work",
      Promise.resolve<Verdict>([
        seal.passed,
        `hash is below the target for ${fmt(seal.work ?? 0)} hashes`,
      ]),
      `nonce ${fmt(block.nonce)}`,
    );
  }
  checkRow(
    facts,
    "Merkle root",
    audit.contents.then((ok): Verdict => [
      ok,
      `recomputed from the ${block.transactions.length} transactions`,
    ]),
    el("span", { className: "mono", textContent: short(block.merkle_root) }),
  );
  if (audit.signatures) {
    checkRow(
      facts,
      "Signatures",
      audit.signatures.then(({ verified, of }): Verdict => [
        verified === of,
        `${verified} of ${of} verified here`,
      ]),
    );
  }
  if (audit.payout) {
    const { passed, reward, fees, amount, to } = audit.payout;
    checkRow(
      facts,
      "Payout",
      Promise.resolve<Verdict>([
        passed,
        `${fmt(reward)} reward + ${fmt(fees)} in fees = ${fmt(amount)}`,
      ]),
      "to ",
      who(to),
    );
  }
  factRow(
    facts,
    "Confirmations",
    fmt(context.height - block.index + 1),
    isTip ? " (the tip)" : "",
  );

  const nav = el(
    "div",
    { className: "pager" },
    block.index
      ? link(href.block(block.index - 1), `← #${block.index - 1}`, "")
      : el("span"),
    isTip
      ? el("span")
      : link(href.block(block.index + 1), `#${block.index + 1} →`, ""),
  );

  const rows: TableEntry[] = block.transactions.map((tx, position) => ({
    transaction: tx,
    status: "confirmed",
    block: { height: block.index, position },
  }));
  return [
    el(
      "h1",
      {},
      block.index ? `Block #${block.index}` : "Genesis",
      " ",
      el("span", {
        className: `badge-kind ${block.validator ? "pos" : "pow"}`,
        textContent: block.validator ? "forged" : "mined",
      }),
    ),
    nav,
    panel("Why it's valid", facts),
    panel(
      `Transactions (${block.transactions.length})`,
      rows.length
        ? txTable(rows, true)
        : el("p", {
            className: "hint",
            textContent: "None: genesis carries no transactions.",
          }),
    ),
  ];
}
