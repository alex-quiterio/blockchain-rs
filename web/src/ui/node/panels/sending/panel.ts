// The send panel and what's waiting: sending from a wallet, staking or unstaking, with what it
// costs and whether the fee gets it into the next block; and the mempool.

import { send } from "@application/sending";
import type { Fees } from "@application/ports";
import { STAKE, UNSTAKE, feesOf } from "@domain/transfers";
import { $, el, fmt, toast, type Child } from "@ui/dom";
import type { NodePage } from "@ui/node/page";
import { fillWalletSelect } from "@ui/node/parties";
import { txView } from "@ui/node/transfers";

const input = (id: string) => $<HTMLInputElement>(id);
const select = (id: string) => $<HTMLSelectElement>(id);

function renderPending({ view, wallets }: NodePage): void {
  const { pending } = view.state;
  const fees = feesOf(pending);
  $("pending-fees").textContent = fees
    ? `· ${fmt(fees)} in fees for the next block`
    : "";
  $("pending").replaceChildren(
    ...(pending.length
      ? pending.map((tx) => txView(wallets, tx))
      : [
          el("li", {
            className: "empty",
            textContent: "Nothing waiting to be added.",
          }),
        ]),
  );
}

/** Offers each wallet to send from, and each wallet, the stake or another address to send to. */
function renderChoices({ view, wallets }: NodePage): void {
  fillWalletSelect(select("send-from"), wallets);
  const extras: [string, string][] = view.isStake()
    ? [
        ["__stake", "Stake: lock coins to validate"],
        ["__unstake", "Unstake: unlock staked coins"],
      ]
    : [];
  fillWalletSelect(select("send-to"), wallets, [
    ...extras,
    ["__other", "Other address…"],
  ]);
}

function updateSendSummary({ view }: NodePage): void {
  const { state } = view;
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
    ...feeAdvice(state.fees, fee),
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
function feeAdvice(fees: Fees | null, fee: number): (Node | string)[] {
  const needed = fees?.room === 0 ? fees.fee : 0;
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

/** Wires up sending, and draws the choices, their cost and what's waiting each time the view changes. */
export function installSending(page: NodePage): void {
  const { node, wallets, actions } = page;
  page.view.onChange(() => {
    renderChoices(page);
    updateSendSummary(page);
    renderPending(page);
  });
  for (const id of ["send-from", "send-to", "send-amount", "send-fee"])
    $(id).oninput = () => updateSendSummary(page);
  $<HTMLFormElement>("send-form").onsubmit = (e) => {
    e.preventDefault();
    void actions.busy($<HTMLButtonElement>("send"), async () => {
      const from = wallets.byAddress(select("send-from").value);
      if (!from) throw new Error("Create a wallet first");
      actions.requireUnlocked();
      const to = recipient(select("send-to").value);
      if (!to) return;
      await send(node, wallets, {
        from,
        to,
        amount: Number(input("send-amount").value),
        fee: Number(input("send-fee").value) || 0,
      });
      toast(`Queued from ${from.name}. It confirms in the next block.`);
    });
  };
}
