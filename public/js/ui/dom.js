// Building the page: elements, numbers, hashes and times as people read them, and notices.

export const $ = (id) => document.getElementById(id);

export function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  Object.assign(node, attrs);
  node.append(...children.flat().filter((c) => c != null && c !== false));
  return node;
}

export const short = (hex) => (hex.length > 16 ? `${hex.slice(0, 8)}…${hex.slice(-6)}` : hex);
export const fmt = (n) => Number(n).toLocaleString();

const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
export function ago(timestamp) {
  const s = Math.round((new Date(timestamp) - Date.now()) / 1000);
  for (const [unit, size] of [["day", 86400], ["hour", 3600], ["minute", 60]]) {
    if (Math.abs(s) >= size) return rtf.format(Math.round(s / size), unit);
  }
  return Math.abs(s) < 5 ? "just now" : rtf.format(s, "second");
}

/** A notice in the corner that goes away on its own. */
export function toast(message, { error = false, ms = 3800 } = {}) {
  const note = el("div", { className: `toast${error ? " error" : ""}`, textContent: message });
  $("toasts").append(note);
  setTimeout(() => note.remove(), ms);
}

/** A question that opens into numbered answers. */
export function explain(question, ...items) {
  return el("details", { className: "explain" }, el("summary", { textContent: question }),
    el("ol", {}, items.map((html) => el("li", { innerHTML: html }))));
}
