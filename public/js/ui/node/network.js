// The network panel: this node's peers and whether they're ahead, behind or in step, adding one
// and syncing now; and the footer, saying which build the node runs.

import { addPeer, syncNow } from "../../application/peers.js";
import { state } from "../../application/node_view.js";
import { $, ago, el, toast } from "../dom.js";
import { busy } from "./actions.js";

export function renderPeers() {
  $("peer-count").textContent = state.peers.length || "";
  const ours = state.height + 1;
  $("peers").replaceChildren(
    ...(state.peers.length
      ? state.peers.map((p) => {
        const known = p.height != null;
        const diff = known ? p.height - ours : 0;
        const rel = !known ? "not reached yet" : diff > 0 ? `${diff} ahead` : diff < 0 ? `${-diff} behind` : "in step";
        return el("li", { className: "peer" },
          el("span", { className: `dot ${p.error ? "down" : known ? "up" : ""}` }),
          el("span", { className: "url", textContent: p.url, title: p.url }),
          el("span", { className: `rel${diff > 0 ? " ahead" : ""}`, textContent: rel }),
          p.error
            ? el("span", { className: "note err", textContent: p.error })
            : p.last_seen ? el("span", { className: "note", textContent: `height ${p.height - 1} · seen ${ago(p.last_seen)}` }) : null);
      })
      : [el("li", { className: "empty", textContent: "No peers. This node is on its own. Add another node's URL to sync with it." })]),
  );
}

export function renderFooter() {
  const n = state.node;
  if (!n) return;
  const dirty = n.commit.endsWith("-dirty");
  $("footer").replaceChildren(
    el("span", {}, "node ", el("span", { className: "mono", textContent: `v${n.version}` })),
    "·",
    el("span", { title: dirty ? "built with uncommitted changes" : "" }, "commit ",
      el("span", { className: `mono${dirty ? " dirty" : ""}`, textContent: n.commit })),
    "·",
    el("span", { textContent: `up since ${new Date(n.started_at).toLocaleString()}` }),
  );
}

export function installNetwork() {
  $("peer-form").onsubmit = (e) => {
    e.preventDefault();
    busy($("add-peer"), async () => {
      const url = $("peer-url").value.trim();
      if (!url) return;
      const { added } = await addPeer(url);
      $("peer-url").value = "";
      toast(added ? `Added ${url}. It now knows this node too.` : `${url} is already a peer`);
    });
  };
  $("sync").onclick = (e) =>
    busy(e.target, async () => {
      const report = await syncNow();
      toast(report.adopted_from
        ? `Adopted the heavier chain from ${report.adopted_from}`
        : `Reached ${report.reached} peer${report.reached === 1 ? "" : "s"}. This chain is already the heaviest.`);
    });
}
