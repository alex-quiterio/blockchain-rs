// The send panel and what's waiting: sending from a wallet, staking or unstaking, with what it
// costs and whether the fee gets it into the next block; and the mempool.

import { state } from "../../../../application/node_view.js";
import { send } from "../../../../application/sending.js";
import { wallets } from "../../../../application/wallets.js";
import { STAKE, UNSTAKE, feesOf } from "../../../../domain/transfers.js";
import { $, el, fmt, toast } from "../../../dom.js";
import { busy, requireUnlocked } from "../../actions.js";
import { txView } from "../../components.js";

export function renderPending() {
  const fees = feesOf(state.pending);
  $("pending-fees").textContent = fees ? `· ${fmt(fees)} in fees for the next block` : "";
  $("pending").replaceChildren(
    ...(state.pending.length
      ? state.pending.map(txView)
      : [el("li", { className: "empty", textContent: "Nothing waiting to be added." })]),
  );
}

export function updateSendSummary() {
  const to = $("send-to").value;
  const amount = Number($("send-amount").value) || 0;
  const fee = Number($("send-fee").value) || 0;
  const from = state.accounts[$("send-from").value];
  const what = to === "__stake" ? "locks" : to === "__unstake" ? "unlocks" : "sends";
  const cost = to === "__unstake" ? fee : amount + fee;
  const parts = [`${what} `, el("b", { textContent: fmt(amount) })];
  if (fee) parts.push(" + ", el("b", { textContent: fmt(fee) }), " fee");
  parts.push(` · costs ${fmt(cost)} of `, el("b", { textContent: fmt(from?.balance ?? 0) }));
  $("send-summary").replaceChildren(...parts, ...feeAdvice(fee));
  const needed = state.fees?.fee ?? 0;
  $("send-fee").placeholder = needed ? `${fmt(needed)} gets in now` : "optional";
}

/**
 * Whether `fee` wins a place in the next block. Only said once it's full: while there's room,
 * any fee gets in. The node's estimate is sure for a sender with nothing else waiting.
 */
function feeAdvice(fee) {
  const needed = state.fees?.room === 0 ? state.fees.fee : 0;
  if (!needed) return [];
  return fee >= needed
    ? [" · ", el("span", { className: "ok", textContent: "enough for the next block" })]
    : [" · ", el("span", { className: "bad" }, "the next block is full: ", el("b", { textContent: fmt(needed) }), " gets in")];
}

/** Where the "To" choice sends: a wallet's address, into or out of the stake, or an address asked for. */
function recipient(choice) {
  if (choice === "__stake") return STAKE;
  if (choice === "__unstake") return UNSTAKE;
  if (choice === "__other") return (prompt("Recipient address (hex)") || "").trim();
  return choice;
}

export function installSending() {
  for (const id of ["send-from", "send-to", "send-amount", "send-fee"]) $(id).oninput = updateSendSummary;
  $("send-form").onsubmit = (e) => {
    e.preventDefault();
    busy($("send"), async () => {
      const from = wallets().byAddress($("send-from").value);
      if (!from) throw new Error("Create a wallet first");
      requireUnlocked();
      const to = recipient($("send-to").value);
      if (!to) return;
      await send({ from, to, amount: Number($("send-amount").value), fee: Number($("send-fee").value) || 0 });
      toast(`Queued from ${from.name}. It confirms in the next block.`);
    });
  };
}
