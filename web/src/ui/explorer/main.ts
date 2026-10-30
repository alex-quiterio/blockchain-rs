// The explorer, put together: every block, transaction and address the node knows, with each of
// the node's checks redone in this page. Routes live in the URL hash, so any view can be linked
// to. The adapters are wired into the application here and nowhere else.

import { refreshContext, search } from "@application/lookup";
import { wire } from "@application/ports";
import { NodeHttpClient } from "@infra/node_client";
import { $, el, toast } from "@ui/dom";
import { href, link, panel } from "./components.ts";
import { addressPage } from "@ui/explorer/pages/address/page";
import { blockPage } from "@ui/explorer/pages/block/page";
import { home } from "@ui/explorer/pages/home/page";
import { txPage } from "@ui/explorer/pages/transaction/page";

const POLL_MS = 4000;

wire({ node: NodeHttpClient });

/** Each route's page folder, which names the part its styles are scoped to. */
const PAGE_FOLDERS: Record<string, string> = {
  block: "block",
  tx: "transaction",
  address: "address",
};

const pages: Record<string, (id: string) => Promise<HTMLElement[]>> = {
  block: blockPage,
  tx: txPage,
  address: addressPage,
};

async function render(): Promise<void> {
  const [, kind, id] =
    location.hash.match(/^#\/(block|tx|address)\/(.+)$/) ?? [];
  const view = $("view");
  // Each page's styles apply only to it, through this: the page's folder under pages/.
  view.dataset.part = (id && kind && PAGE_FOLDERS[kind]) || "home";
  try {
    const page = kind && id ? pages[kind] : undefined;
    const parts =
      page && id ? await page(decodeURIComponent(id)) : await home();
    view.replaceChildren(...parts);
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    view.replaceChildren(
      panel(
        "Not found",
        el("p", { className: "hint", textContent: message }),
        el("p", {}, link("#/", "← Back to the latest blocks", "")),
      ),
    );
  }
}

$("search").addEventListener("submit", async (event) => {
  event.preventDefault();
  try {
    const found = await search($<HTMLInputElement>("query").value);
    location.hash = href[found.kind](found.id);
  } catch (e) {
    toast(e instanceof Error ? e.message : String(e), {
      error: true,
      ms: 5000,
    });
  }
});
window.addEventListener("hashchange", () => {
  void render();
  window.scrollTo(0, 0);
});

await refreshContext();
await render();
// New blocks change confirmations, the mempool and the latest blocks, so redraw on each one.
setInterval(async () => {
  if (await refreshContext().catch(() => false)) await render();
}, POLL_MS);
