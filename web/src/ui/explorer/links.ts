// Links between the explorer's views: each block, transaction and address has its own, and the
// special addresses get a name instead.

import { COINBASE, STAKE, UNSTAKE } from "@domain/transfers";
import { el, short } from "@ui/dom";

export const href = {
  block: (id: string | number): string => `#/block/${id}`,
  tx: (sig: string): string => `#/tx/${sig}`,
  address: (a: string): string => `#/address/${a}`,
};
export const link = (
  to: string,
  text: string,
  className = "mono",
): HTMLAnchorElement => el("a", { href: to, className, textContent: text });

/** An address as a link, or the special names it can take. */
export function who(address: string): HTMLElement {
  if (address === COINBASE)
    return el("span", { className: "tag", textContent: "new coins" });
  if (address === STAKE || address === UNSTAKE)
    return el("span", { className: `tag ${address}`, textContent: address });
  return link(href.address(address), short(address));
}
