// The send panel and what's waiting: sending from a wallet, staking or unstaking, with what it
// costs and whether the fee gets it into the next block; and the mempool.

import { state } from "@application/node_view";
import { send } from "@application/sending";
import { wallets } from "@application/wallets";
import { STAKE, UNSTAKE, feesOf } from "@domain/transfers";
import { $, el, fmt, toast, type Child } from "@ui/dom";
import { busy, requireUnlocked } from "@ui/node/actions";
import { txView } from "@ui/node/components";

const input = (id: string) => $<HTMLInputElement>(id);
const select = (id: string) => $<HTMLSelectElement>(id);

export function renderPending(): void {
  const fees = feesOf(state.pending);
  $("pending-fees").textContent = fees
    ? `· ${fmt(fees)} in fees for the next block`
    : "";
  $("pending").replaceChildren(
    ...(state.pending.length
      ? state.pending.map(txView)
      : [
          el("li", {
            className: "empty",
            textContent: "Nothing waiting to be added.",
          }),
        ]),
  );
}

export function updateSendSummary(): void {
  const to = select("send-to").value;
  const amount = Number(input("send-amount").value) || 0;
  const fee = Number(input("send-fee").value) || 0;
  const from = state.accounts[select("send-from").value];
  const what =
    to === "__stake" ? "locks" : to === "__unstake" ? "unlocks" : "sends";
  const cost = to === "__unstake" ? fee : amount + fee;
  const parts: Child[] = [`${what} `, el("b", { textContent: fmt(amount) })];
  if (fee) parts.push(" + ", el("b", { textContent: fmt(fee) }), " fee");
  parts.push(
    ` · costs ${fmt(cost)} of `,
    el("b", { textContent: fmt(from?.balance ?? 0) }),
  );
  $("send-summary").replaceChildren(
    ...(parts.flat() as (Node | string)[]),
    ...feeAdvice(fee),
  );
  const needed = state.fees?.fee ?? 0;
  input("send-fee").placeholder = needed
    ? `${fmt(needed)} gets in now`
    : "optional";
}

/**
 * Whether `fee` wins a place in the next block. Only said once it's full: while there's room,
 * any fee gets in. The node's estimate is sure for a sender with nothing else waiting.
 */
function feeAdvice(fee: number): (Node | string)[] {
  const needed = state.fees?.room === 0 ? state.fees.fee : 0;
  if (!needed) return [];
  return fee >= needed
    ? [
        " · ",
        el("span", {
          className: "ok",
          textContent: "enough for the next block",
        }),
      ]
    : [
        " · ",
        el(
          "span",
          { className: "bad" },
          "the next block is full: ",
          el("b", { textContent: fmt(needed) }),
          " gets in",
        ),
      ];
}

/**
 * Where the "To" choice sends: a wallet's address, into or out of the stake, or an address asked
 * for.
 */
function recipient(choice: string): string {
  if (choice === "__stake") return STAKE;
  if (choice === "__unstake") return UNSTAKE;
  if (choice === "__other")
    return (prompt("Recipient address (hex)") || "").trim();
  return choice;
}

export function installSending(): void {
  for (const id of ["send-from", "send-to", "send-amount", "send-fee"])
    $(id).oninput = updateSendSummary;
  $<HTMLFormElement>("send-form").onsubmit = (e) => {
    e.preventDefault();
    void busy($<HTMLButtonElement>("send"), async () => {
      const from = wallets().byAddress(select("send-from").value);
      if (!from) throw new Error("Create a wallet first");
      requireUnlocked();
      const to = recipient(select("send-to").value);
      if (!to) return;
      await send({
        from,
        to,
        amount: Number(input("send-amount").value),
        fee: Number(input("send-fee").value) || 0,
      });
      toast(`Queued from ${from.name}. It confirms in the next block.`);
    });
  };
}
