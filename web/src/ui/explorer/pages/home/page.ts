// The explorer's home: where the network stands, the latest blocks and what's waiting.

import type { Lookup } from "@application/lookup";
import { feesOf } from "@domain/transfers";
import { ago, el, fmt, short } from "@ui/dom";
import { factRow, panel } from "@ui/explorer/facts";
import { href, link, who } from "@ui/explorer/links";
import { table, txTable } from "@ui/explorer/tables";

const LATEST = 20;

export async function home(lookup: Lookup): Promise<HTMLElement[]> {
  const { status, blocks, pending } = await lookup.latest(LATEST);
  const rules = lookup.rules();
  const stake = rules.kind === "proof_of_stake";

  const facts = el("dl", { className: "facts" });
  factRow(facts, "Consensus", stake ? "proof of stake" : "proof of work");
  factRow(facts, "Height", link(href.block(status.height), fmt(status.height)));
  factRow(facts, "Tip", link(href.block(status.tip), short(status.tip)));
  factRow(
    facts,
    "Weight",
    fmt(status.weight),
    stake ? " stake behind the blocks" : " expected hashes behind the blocks",
  );
  const work = rules.work;
  if (work)
    factRow(facts, "Next block", `needs ${fmt(work)} hashes on average`);

  const rows = blocks
    .slice()
    .reverse()
    .map((block) => {
      const [payout, ...transfers] = block.transactions;
      return el(
        "tr",
        {},
        el("td", {}, link(href.block(block.index), `#${block.index}`)),
        el("td", {}, link(href.block(block.hash), short(block.hash))),
        el("td", {
          className: "muted",
          textContent: block.index ? ago(block.timestamp) : "the beginning",
        }),
        el("td", { className: "num", textContent: fmt(transfers.length) }),
        el("td", { className: "num", textContent: fmt(feesOf(transfers)) }),
        el("td", {}, payout ? who(payout.to) : "–"),
      );
    });

  return [
    panel("Network", facts),
    panel(
      `Latest blocks`,
      table(["Height", "Hash", "When", "Transfers", "Fees", "Paid to"], rows),
    ),
    panel(
      `Mempool (${pending.length})`,
      pending.length
        ? txTable(pending.map((tx) => ({ transaction: tx, status: "pending" })))
        : el("p", {
            className: "hint",
            textContent:
              "Nothing waiting. Transfers sit here until a block takes them.",
          }),
    ),
  ];
}
