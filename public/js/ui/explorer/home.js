// The explorer's home: where the network stands, the latest blocks and what's waiting.

import { context, latest } from "../../application/lookup.js";
import { feesOf } from "../../domain/transfers.js";
import { ago, el, fmt, short } from "../dom.js";
import { factRow, href, link, panel, table, txTable, who } from "./components.js";

const LATEST = 20;

export async function home() {
  const { status, blocks, pending } = await latest(LATEST);
  const stake = context.consensus.kind === "proof_of_stake";

  const facts = el("dl", { className: "facts" });
  factRow(facts, "Consensus", stake ? "proof of stake" : "proof of work");
  factRow(facts, "Height", link(href.block(status.height), fmt(status.height)));
  factRow(facts, "Tip", link(href.block(status.tip), short(status.tip)));
  factRow(facts, "Weight", fmt(status.weight), stake ? " stake behind the blocks" : " expected hashes behind the blocks");
  if (context.consensus.work) factRow(facts, "Next block", `needs ${fmt(context.consensus.work)} hashes on average`);

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
