// What a page says: panels of facts, each one plain or a check whose verdict arrives once it
// finishes.

import { el, messageOf, type Child } from "@ui/dom";

export const ok = (text: string): HTMLSpanElement =>
  el("span", { className: "ok", textContent: `✓ ${text}` });
export const bad = (text: string): HTMLSpanElement =>
  el("span", { className: "bad", textContent: `✗ ${text}` });

/** A check's verdict: whether it passed, and what it says. */
export type Verdict = [passed: boolean, text: string];

/** A row of the facts list whose verdict arrives later. */
export function checkRow(
  facts: HTMLElement,
  label: string,
  check: Promise<Verdict>,
  ...detail: Child[]
): void {
  const verdict = el("span", { className: "muted", textContent: "checking…" });
  check
    .then(([passed, text]) =>
      verdict.replaceWith(passed ? ok(text) : bad(text)),
    )
    .catch((e: unknown) => verdict.replaceWith(bad(messageOf(e))));
  facts.append(
    el("dt", { textContent: label }),
    el("dd", {}, verdict, detail.length ? " " : null, detail),
  );
}

export function factRow(
  facts: HTMLElement,
  label: string,
  ...value: Child[]
): void {
  facts.append(el("dt", { textContent: label }), el("dd", {}, value));
}

export const panel = (title: string, ...children: Child[]): HTMLElement =>
  el("section", { className: "panel" }, el("h2", {}, title), children);
export const formula = (text: string): HTMLSpanElement =>
  el("span", { className: "formula", textContent: text });
