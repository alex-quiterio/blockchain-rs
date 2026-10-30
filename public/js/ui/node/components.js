// Small views several panels draw: a party with its avatar, a transfer, a hash with its zeros.

import { isStake } from "../../application/node_view.js";
import { wallets } from "../../application/wallets.js";
import { kindOf } from "../../domain/transfers.js";
import { el, fmt, short } from "../dom.js";

export const nameOf = (address) => wallets().byAddress(address)?.name || short(address);
export const pct = (part, whole) => `${whole ? Math.round((part / whole) * 100) : 0}%`;

export function hues(address) {
  const h1 = parseInt(address.slice(0, 4), 16) % 360;
  return [h1, (h1 + 40 + (parseInt(address.slice(4, 6), 16) % 120)) % 360];
}

/** A gradient tile derived from the address bytes, so the same address always looks the same. */
export function avatar(address, small = false) {
  const [h1, h2] = hues(address);
  const tile = el("span", { className: `avatar${small ? " sm" : ""}` });
  tile.style.background = `linear-gradient(135deg, hsl(${h1} 75% 58%), hsl(${h2} 70% 42%))`;
  return tile;
}

/** "65,536", or "4.3M" once it gets long. */
export const fmtWork = (work) => (work >= 1e6
  ? new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 }).format(work)
  : fmt(work));

/** The leading hex zeros a given amount of work guarantees: 16^d hashes means d zeros. */
const zerosFor = (work) => Math.max(0, Math.floor(Math.log(work) / Math.log(16) + 1e-9));

/** A hash, with the leading zeros its proof of work guarantees picked out. */
export function hashView(hash, work) {
  const zeros = hash.match(/^0*/)[0].length;
  const shown = isStake() || !work ? 0 : Math.min(zeros, zerosFor(work));
  return el("div", { className: "hash", title: hash },
    el("span", { className: "zeros", textContent: hash.slice(0, shown) }), hash.slice(shown));
}

export const party = (address) =>
  el("span", { className: "party" }, avatar(address, true), el("span", { textContent: nameOf(address) }));
const arrow = () => el("span", { className: "arrow", textContent: "→" });
const feeNote = (tx) => (tx.fee ? el("span", { className: "fee", textContent: `fee ${fmt(tx.fee)}` }) : null);

export function txView(tx) {
  const kind = kindOf(tx);
  if (kind === "coinbase") {
    return el("li", { className: "tx coinbase" },
      el("span", { className: "tag", textContent: "payout" }), arrow(), party(tx.to),
      el("span", { className: "amt", textContent: `+${fmt(tx.amount)}` }));
  }
  if (kind === "stake" || kind === "unstake") {
    return el("li", { className: "tx", title: `nonce ${tx.nonce}` },
      party(tx.from), el("span", { className: `tag ${kind}`, textContent: kind === "stake" ? "stakes" : "unstakes" }),
      feeNote(tx), el("span", { className: "amt", textContent: fmt(tx.amount) }));
  }
  return el("li", { className: "tx", title: `nonce ${tx.nonce}` },
    party(tx.from), arrow(), party(tx.to), feeNote(tx), el("span", { className: "amt", textContent: fmt(tx.amount) }));
}
