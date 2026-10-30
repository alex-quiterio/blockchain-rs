// A block's page: why it's valid, each of the node's checks redone here, and what it holds.

import { block as lookUpBlock, context, rules } from "@application/lookup";
import { feesOf } from "@domain/transfers";
import { headerHash, meetsWork } from "@domain/verification/blocks";
import { root } from "@domain/verification/merkle";
import {
  blockIsSigned,
  transferIsSigned,
} from "@domain/verification/signatures";
import { ago, el, fmt, short } from "@ui/dom";
import {
  checkRow,
  factRow,
  formula,
  href,
  link,
  panel,
  txTable,
  who,
  type TableEntry,
  type Verdict,
} from "@ui/explorer/components";

export async function blockPage(id: string): Promise<HTMLElement[]> {
  const { block, prev, work } = await lookUpBlock(id);
  const [payout, ...transfers] = block.transactions;
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
    headerHash(block).then((h): Verdict => [
      h === block.hash,
      "recomputed here and matches",
    ]),
    formula(
      "sha256(index | timestamp | prev_hash | merkle_root | nonce" +
        (block.validator ? " | validator | reveal" : "") +
        ")",
    ),
  );
  if (prev) {
    checkRow(
      facts,
      "Link",
      Promise.resolve<Verdict>([
        prev.hash === block.prev_hash,
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
  if (block.validator) {
    checkRow(
      facts,
      "Proof",
      blockIsSigned(block).then((v): Verdict => [
        v,
        "the validator's signature over the hash",
      ]),
      "by ",
      who(block.validator),
    );
  } else {
    checkRow(
      facts,
      "Proof of work",
      Promise.resolve<Verdict>([
        work != null && meetsWork(block.hash, work),
        `hash is below the target for ${fmt(work ?? 0)} hashes`,
      ]),
      `nonce ${fmt(block.nonce)}`,
    );
  }
  checkRow(
    facts,
    "Merkle root",
    root(block.transactions).then((r): Verdict => [
      r === block.merkle_root,
      `recomputed from the ${block.transactions.length} transactions`,
    ]),
    el("span", { className: "mono", textContent: short(block.merkle_root) }),
  );
  if (transfers.length) {
    checkRow(
      facts,
      "Signatures",
      Promise.all(transfers.map(transferIsSigned)).then((results): Verdict => [
        results.every(Boolean),
        `${results.filter(Boolean).length} of ${results.length} verified here`,
      ]),
    );
  }
  if (payout) {
    const fees = feesOf(transfers);
    const reward = rules().reward;
    checkRow(
      facts,
      "Payout",
      Promise.resolve<Verdict>([
        payout.amount === reward + fees,
        `${fmt(reward)} reward + ${fmt(fees)} in fees = ${fmt(payout.amount)}`,
      ]),
      "to ",
      who(payout.to),
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
