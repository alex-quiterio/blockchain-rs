// An address's page: what it holds, its history, and whether that history adds up to it.

import { address as lookUpAddress } from "../../../../application/lookup.js";
import { COINBASE, STAKE, UNSTAKE, kindOf } from "../../../../domain/transfers.js";
import { el, fmt, short } from "../../../dom.js";
import { checkRow, factRow, panel, txTable } from "../../components.js";

export async function addressPage(address) {
  const { account, history } = await lookUpAddress(address);
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
