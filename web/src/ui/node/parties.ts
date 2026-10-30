// Who a transfer is between, as the node page draws them: a wallet's name if it's one of ours,
// an avatar every address keeps, and a choice among our wallets.

import type { Wallets } from "@domain/wallets/wallets";
import { el, short } from "@ui/dom";

export const nameOf = (wallets: Wallets, address: string): string =>
  wallets.byAddress(address)?.name || short(address);

export function hues(address: string): [number, number] {
  const h1 = parseInt(address.slice(0, 4), 16) % 360;
  return [h1, (h1 + 40 + (parseInt(address.slice(4, 6), 16) % 120)) % 360];
}

/** A gradient tile derived from the address bytes, so the same address always looks the same. */
export function avatar(address: string, small = false): HTMLSpanElement {
  const [h1, h2] = hues(address);
  const tile = el("span", { className: `avatar${small ? " sm" : ""}` });
  tile.style.background = `linear-gradient(135deg, hsl(${h1} 75% 58%), hsl(${h2} 70% 42%))`;
  return tile;
}

export const party = (wallets: Wallets, address: string): HTMLSpanElement =>
  el(
    "span",
    { className: "party" },
    avatar(address, true),
    el("span", { textContent: nameOf(wallets, address) }),
  );

/** Fills a select with the wallets, then `extras` as `[value, label]`, keeping what was chosen. */
export function fillWalletSelect(
  select: HTMLSelectElement,
  wallets: Wallets,
  extras: [string, string][] = [],
): void {
  const current = select.value;
  select.replaceChildren(
    ...wallets.list.map((w) =>
      el("option", { value: w.address, textContent: w.name }),
    ),
    ...extras.map(([value, label]) =>
      el("option", { value, textContent: label }),
    ),
  );
  if ([...select.options].some((o) => o.value === current))
    select.value = current;
}
