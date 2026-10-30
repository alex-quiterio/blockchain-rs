// The explorer, put together: every block, transaction and address the node knows, with each of
// the node's checks redone in this page. Routes live in the URL hash, so any view can be linked
// to. The adapters are wired into the application here and nowhere else.

import { refreshContext, search } from "../../application/lookup.js";
import { wire } from "../../application/ports.js";
import { NodeHttpClient } from "../../infrastructure/node_client.js";
import { $, el, toast } from "../dom.js";
import { addressPage } from "./address.js";
import { blockPage } from "./block.js";
import { href, link, panel } from "./components.js";
import { home } from "./home.js";
import { txPage } from "./transaction.js";

const POLL_MS = 4000;

wire({ node: NodeHttpClient });

async function render() {
  const [, kind, id] = location.hash.match(/^#\/(block|tx|address)\/(.+)$/) || [];
  const view = $("view");
  try {
    const parts = await ({ block: blockPage, tx: txPage, address: addressPage }[kind] || home)(id && decodeURIComponent(id));
    view.replaceChildren(...parts);
  } catch (e) {
    view.replaceChildren(panel("Not found", el("p", { className: "hint", textContent: e.message }), el("p", {}, link("#/", "← Back to the latest blocks", ""))));
  }
}

$("search").addEventListener("submit", async (event) => {
  event.preventDefault();
  try {
    const found = await search($("query").value);
    location.hash = href[found.kind](found.id);
  } catch (e) {
    toast(e.message, { error: true, ms: 5000 });
  }
});
window.addEventListener("hashchange", () => { render(); window.scrollTo(0, 0); });

await refreshContext();
await render();
// New blocks change confirmations, the mempool and the latest blocks, so redraw on each one.
setInterval(async () => {
  if (await refreshContext().catch(() => false)) render();
}, POLL_MS);
