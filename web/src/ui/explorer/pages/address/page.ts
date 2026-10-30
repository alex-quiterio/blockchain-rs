// An address's page: what it holds, its history, and whether that history adds up to it.

import { auditAddress } from "@application/audit";
import { el, fmt, short } from "@ui/dom";
import {
  checkRow,
  factRow,
  panel,
  txTable,
  type Verdict,
} from "@ui/explorer/components";

export async function addressPage(address: string): Promise<HTMLElement[]> {
  const { account, history, totals, addsUp } = await auditAddress(address);
  const { rewards, received, sent, fees, staked } = totals;

  const facts = el("dl", { className: "facts" });
  factRow(
    facts,
    "Address",
    el("span", { className: "mono wrap", textContent: address }),
    " · also its ed25519 public key",
  );
  factRow(
    facts,
    "Balance",
    el("b", { textContent: fmt(account.balance) }),
    account.staked ? ` · ${fmt(account.staked)} staked` : "",
  );
  factRow(
    facts,
    "In",
    `${fmt(rewards)} from block rewards, ${fmt(received)} from transfers`,
  );
  factRow(
    facts,
    "Out",
    `${fmt(sent)} sent, ${fmt(fees)} in fees`,
    staked ? `, ${fmt(staked)} locked as stake` : "",
  );
  checkRow(
    facts,
    "Adds up",
    Promise.resolve<Verdict>([
      addsUp,
      "in − out − stake = balance, from its history alone",
    ]),
  );
  factRow(
    facts,
    "Next nonce",
    fmt(account.next_nonce),
    " · its next transfer must carry this number",
  );

  return [
    el(
      "h1",
      {},
      "Address ",
      el("span", { className: "mono muted", textContent: short(address) }),
    ),
    panel("Account", facts),
    panel(
      `History (${history.length})`,
      history.length
        ? txTable(history, false, address)
        : el("p", {
            className: "hint",
            textContent:
              "Nothing yet. An address appears on the chain only once coins move to or from it.",
          }),
    ),
  ];
}
