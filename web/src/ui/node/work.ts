// Proof of work as the node page shows it: how much work, and a hash with the leading zeros that
// work guarantees picked out.

import { el, fmt } from "@ui/dom";

/** "65,536", or "4.3M" once it gets long. */
export const fmtWork = (work: number): string =>
  work >= 1e6
    ? new Intl.NumberFormat(undefined, {
        notation: "compact",
        maximumFractionDigits: 1,
      }).format(work)
    : fmt(work);

/** The leading hex zeros a given amount of work guarantees: 16^d hashes means d zeros. */
const zerosFor = (work: number): number =>
  Math.max(0, Math.floor(Math.log(work) / Math.log(16) + 1e-9));

/** A hash, with the leading zeros its proof of work guarantees picked out; none without work. */
export function hashView(
  hash: string,
  work: number | undefined,
): HTMLDivElement {
  const zeros = (hash.match(/^0*/)?.[0] ?? "").length;
  const shown = work ? Math.min(zeros, zerosFor(work)) : 0;
  return el(
    "div",
    { className: "hash", title: hash },
    el("span", { className: "zeros", textContent: hash.slice(0, shown) }),
    hash.slice(shown),
  );
}
