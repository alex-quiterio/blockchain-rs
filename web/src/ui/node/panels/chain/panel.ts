// The chain panel: the stats along the top, and the latest blocks, each with the story of why
// it's valid, its Merkle root re-checked here.

import {
  SHOWN_BLOCKS,
  blockAt,
  consensus,
  isStake,
  state,
  supply,
  tip,
} from "@application/node_view";
import type { Block } from "@domain/blocks";
import { feesOf, type Transfer } from "@domain/transfers";
import { root } from "@domain/verification/merkle";
import { $, ago, el, fmt, short, type Child } from "@ui/dom";
import { fmtWork, hashView, party, txView } from "@ui/node/components";

const SHOWN_TXS = 6;
let seenTip: string | undefined | null = null;

export function renderStats(): void {
  const c = state.consensus;
  $("stat-consensus").textContent = c
    ? isStake()
      ? "Proof of stake"
      : `Proof of work · ${fmtWork(Number(c.work))}`
    : "–";
  $("stat-height").textContent = state.height >= 0 ? fmt(state.height) : "–";
  $("stat-supply").textContent = fmt(supply());
  $("stat-pending").textContent = fmt(state.pending.length);
  const pill = $("validity");
  pill.className = `pill ${state.valid ? "ok" : "bad"}`;
  pill.lastChild!.textContent = state.valid ? "chain valid" : "chain INVALID";
  pill.title = state.problem ?? "";
}

export function renderChain(): void {
  const fresh = seenTip !== null && tip() !== seenTip;
  seenTip = tip();
  const open = new Set(
    [...document.querySelectorAll<HTMLDetailsElement>(".breakdown[open]")].map(
      (d) => d.dataset.hash,
    ),
  );
  const shown = state.blocks.slice(-SHOWN_BLOCKS).reverse();
  const older = shown.at(-1)?.index ?? 0;
  $("blocks").replaceChildren(
    ...shown.map((block, i) => {
      const view = blockView(block, blockAt(block.index - 1), fresh && i === 0);
      const details = view.querySelector<HTMLDetailsElement>(".breakdown")!;
      details.dataset.hash = block.hash;
      details.open = open.has(block.hash);
      return view;
    }),
    ...(older > 0
      ? [
          el(
            "li",
            { className: "older" },
            el("a", {
              href: `/explorer.html#/block/${older - 1}`,
              textContent: `${fmt(older)} older blocks in the Explorer →`,
            }),
          ),
        ]
      : []),
  );
}

function blockView(
  block: Block,
  prev: Block | undefined,
  fresh: boolean,
): HTMLLIElement {
  const genesis = block.index === 0;
  const [payout, ...rest] = block.transactions;
  const forged = Boolean(block.validator);
  return el(
    "li",
    { className: `block${genesis ? " genesis" : ""}${fresh ? " fresh" : ""}` },
    el("div", { className: "marker", textContent: String(block.index) }),
    el(
      "article",
      { className: "card" },
      el(
        "div",
        { className: "card-head" },
        el("strong", {
          textContent: genesis ? "Genesis" : `Block ${block.index}`,
        }),
        genesis
          ? null
          : el("span", {
              className: "when",
              textContent: ago(block.timestamp),
              title: new Date(block.timestamp).toLocaleString(),
            }),
        genesis
          ? null
          : el("span", {
              className: `badge-kind ${forged ? "pos" : "pow"}`,
              textContent: forged ? "forged" : "mined",
            }),
        el("span", {
          className: "ntx",
          textContent: `${block.transactions.length} tx`,
        }),
      ),
      hashView(block.hash, state.works[block.index]),
      el(
        "div",
        { className: "meta" },
        block.validator
          ? el("span", {}, "forged by ", party(block.validator))
          : el(
              "span",
              {},
              "nonce ",
              el("span", { className: "mono", textContent: fmt(block.nonce) }),
            ),
        el(
          "span",
          {},
          "prev ",
          el("span", {
            className: "mono",
            textContent: short(block.prev_hash),
          }),
        ),
      ),
      txList(block.transactions),
      breakdown(block, prev, payout, rest),
    ),
  );
}

/** A block's transactions; long lists show the first few and fold the rest away. */
function txList(transactions: Transfer[]): HTMLUListElement | null {
  if (!transactions.length) return null;
  const list = el(
    "ul",
    { className: "txs" },
    transactions.slice(0, SHOWN_TXS).map(txView),
  );
  if (transactions.length > SHOWN_TXS) {
    const rest = transactions.slice(SHOWN_TXS);
    const more = el(
      "details",
      { className: "more-txs" },
      el("summary", {
        textContent: `${fmt(rest.length)} more transactions · ${fmt(feesOf(rest))} in fees`,
      }),
      el("ul", { className: "txs" }, rest.map(txView)),
    );
    list.append(el("li", {}, more));
  }
  return list;
}

/** The "why is this block valid" story, one fact per row. */
function breakdown(
  block: Block,
  prev: Block | undefined,
  payout: Transfer | undefined,
  transfers: Transfer[],
): HTMLDetailsElement {
  const facts: HTMLElement[] = [];
  const row = (label: string, ...value: Child[]) =>
    facts.push(el("dt", { textContent: label }), el("dd", {}, value));
  const yes = (text: string) =>
    el("span", { className: "ok", textContent: `✓ ${text}` });

  if (block.index === 0 || !prev) {
    row(
      "Role",
      "The fixed first block. Every node with the same rules builds the same one, so they all start from the same place.",
    );
  } else {
    row(
      "Link",
      yes("prev matches"),
      ` block ${prev.index}'s hash, so changing block ${prev.index} would break this one too.`,
    );
  }
  if (block.validator) {
    row(
      "Proof",
      yes("signed"),
      " by ",
      party(block.validator),
      ". The validator was drawn at random, weighted by stake, from a randomness beacon that only validators' committed reveals can move, so every node agrees whose turn it was and nobody could steer it.",
    );
    if (block.reveal) {
      row(
        "Randomness",
        "revealed ",
        el("span", { className: "formula", textContent: short(block.reveal) }),
        ", the next link of a hash chain the validator committed to when staking. It was fixed in advance, so they had nothing to choose; it's mixed into the beacon that picked the next validator.",
      );
    }
  } else {
    const work = state.works[block.index] ?? 0;
    row(
      "Proof",
      yes(`below the target for ${fmtWork(work)} hashes`),
      ` Found on try ${fmt(block.nonce + 1)}. Each try changes the nonce and rehashes; at this difficulty it takes ${fmtWork(work)} tries on average.`,
    );
    const before = state.works[block.index - 1];
    const { target_block_secs: target, retarget_interval: n } = consensus();
    if (block.index > 0 && before && work !== before && target && n) {
      const [last, first] = [
        blockAt(block.index - 1),
        blockAt(block.index - 1 - n),
      ];
      const took =
        last && first
          ? (new Date(last.timestamp).getTime() -
              new Date(first.timestamp).getTime()) /
            1000
          : NaN;
      if (!Number.isNaN(took))
        row(
          "Difficulty",
          work > before ? "↑ " : "↓ ",
          el("b", { textContent: `${(work / before).toFixed(2)}×` }),
          `: the last ${n} blocks took ${took.toFixed(0)}s against a target of ${target * n}s, so ${work > before ? "mining got harder" : "mining got easier"} (by at most 4×).`,
        );
    }
  }
  if (payout) {
    const fees = feesOf(transfers);
    row(
      "Payout",
      `${fmt(payout.amount - fees)} reward + ${fmt(fees)} in fees = `,
      el("b", { textContent: fmt(payout.amount) }),
      " to ",
      party(payout.to),
    );
  }
  const check = el("span", { textContent: "checking…" });
  root(block.transactions).then((r) =>
    check.replaceWith(
      r === block.merkle_root
        ? yes("match the Merkle root")
        : el("span", { textContent: "✗ don't match the Merkle root" }),
    ),
  );
  const n = block.transactions.length;
  row(
    "Contents",
    check,
    " ",
    el("span", { className: "formula", textContent: short(block.merkle_root) }),
    n > 1
      ? `, recomputed here by hashing the ${n} transactions in pairs up to one hash. The block hash covers the root, so changing any transaction breaks the block, and anyone holding just the header can check one transaction with ${Math.ceil(Math.log2(n))} hashes.`
      : ", recomputed here from the transactions. The block hash covers the root, so changing any transaction breaks the block.",
  );
  const fields = [
    "index",
    "timestamp",
    "prev_hash",
    "merkle_root",
    "nonce",
    block.validator && "validator",
  ].filter(Boolean);
  row(
    "Hash",
    el("span", {
      className: "formula",
      textContent: `sha256(${fields.join(" | ")})`,
    }),
  );
  return el(
    "details",
    { className: "breakdown" },
    el("summary", { textContent: "Breakdown" }),
    el("dl", { className: "facts" }, facts),
  );
}
