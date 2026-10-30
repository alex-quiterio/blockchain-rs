// Building the page: elements, numbers, hashes and times as people read them, and notices.

/** The element with this id, as the kind of element the caller expects. Throws if it's missing. */
export function $<T extends HTMLElement = HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`The page has no #${id}`);
  return found as T;
}

/** Anything an element can hold: nodes and text, nested lists of them, or nothing. */
export type Child = Node | string | null | undefined | false | readonly Child[];

type Tag = keyof HTMLElementTagNameMap;

/** An element with `attrs` set on it, holding `children`. */
export function el<K extends Tag>(
  tag: K,
  attrs: Partial<Omit<HTMLElementTagNameMap[K], "style">> = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  Object.assign(node, attrs);
  const flat = (children as Child[]).flat(Infinity as 1) as Child[];
  node.append(
    ...flat.filter((c): c is Node | string => c != null && c !== false),
  );
  return node;
}

export const short = (hex: string): string =>
  hex.length > 16 ? `${hex.slice(0, 8)}…${hex.slice(-6)}` : hex;
export const fmt = (n: number | string): string => Number(n).toLocaleString();

const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
export function ago(timestamp: string): string {
  const s = Math.round((new Date(timestamp).getTime() - Date.now()) / 1000);
  for (const [unit, size] of [
    ["day", 86400],
    ["hour", 3600],
    ["minute", 60],
  ] as const) {
    if (Math.abs(s) >= size) return rtf.format(Math.round(s / size), unit);
  }
  return Math.abs(s) < 5 ? "just now" : rtf.format(s, "second");
}

/** A notice in the corner that goes away on its own. */
export function toast(
  message: string,
  { error = false, ms = 3800 }: { error?: boolean; ms?: number } = {},
): void {
  const note = el("div", {
    className: `toast${error ? " error" : ""}`,
    textContent: message,
  });
  $("toasts").append(note);
  setTimeout(() => note.remove(), ms);
}

/**
 * Fills `container` from `build`, but only when `key` differs from what it was last built for:
 * a redraw that changes nothing it shows leaves it alone, with what's typed, chosen, focused or
 * open in it.
 */
export function rebuildOn(
  container: HTMLElement,
  key: string,
  build: () => Node[],
): void {
  if (container.dataset.builtFor === key) return;
  container.dataset.builtFor = key;
  container.replaceChildren(...build());
}

/** Whether the first `<details>` in `container` is open, to keep it so across a redraw. */
export const isOpen = (container: HTMLElement): boolean =>
  container.querySelector("details")?.open ?? false;

/** What a refusal or failure says. */
export const messageOf = (err: unknown): string =>
  err instanceof Error ? err.message : String(err);

/** A refusal or failure, shown as an error notice. */
export const showError = (err: unknown, ms?: number): void =>
  toast(messageOf(err), { error: true, ms });

/**
 * A question that opens into numbered answers. Each answer is trusted HTML written in this code.
 */
export function explain(
  question: string,
  ...items: string[]
): HTMLDetailsElement {
  return el(
    "details",
    { className: "explain" },
    el("summary", { textContent: question }),
    el(
      "ol",
      {},
      items.map((html) => el("li", { innerHTML: html })),
    ),
  );
}
