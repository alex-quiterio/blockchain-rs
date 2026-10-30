// A transfer as the node page lists it: who to whom, how much, and its fee.

import { kindOf, type Transfer } from "@domain/transfers";
import type { Wallets } from "@domain/wallets/wallets";
import { el, fmt } from "@ui/dom";
import { party } from "./parties.ts";

const arrow = (): HTMLSpanElement =>
  el("span", { className: "arrow", textContent: "→" });
const feeNote = (tx: Transfer): HTMLSpanElement | null =>
  tx.fee
    ? el("span", { className: "fee", textContent: `fee ${fmt(tx.fee)}` })
    : null;

export function txView(wallets: Wallets, tx: Transfer): HTMLLIElement {
  const kind = kindOf(tx);
  if (kind === "coinbase") {
    return el(
      "li",
      { className: "tx coinbase" },
      el("span", { className: "tag", textContent: "payout" }),
      arrow(),
      party(wallets, tx.to),
      el("span", { className: "amt", textContent: `+${fmt(tx.amount)}` }),
    );
  }
  if (kind === "stake" || kind === "unstake") {
    return el(
      "li",
      { className: "tx", title: `nonce ${tx.nonce}` },
      party(wallets, tx.from),
      el("span", {
        className: `tag ${kind}`,
        textContent: kind === "stake" ? "stakes" : "unstakes",
      }),
      feeNote(tx),
      el("span", { className: "amt", textContent: fmt(tx.amount) }),
    );
  }
  return el(
    "li",
    { className: "tx", title: `nonce ${tx.nonce}` },
    party(wallets, tx.from),
    arrow(),
    party(wallets, tx.to),
    feeNote(tx),
    el("span", { className: "amt", textContent: fmt(tx.amount) }),
  );
}
