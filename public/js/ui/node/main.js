// The node page, put together: the adapters are wired into the application here and nowhere
// else, then each panel is installed and drawn, and the node is polled for changes made
// elsewhere: other tabs, other nodes syncing, `POST /chain`.

import { hasMoved, refreshView } from "../../application/node_view.js";
import { wire } from "../../application/ports.js";
import { NodeHttpClient } from "../../infrastructure/node_client.js";
import { WalletsLocalStorageRepository } from "../../infrastructure/repository/local_storage.js";
import { toast } from "../dom.js";
import { isWorking, onRefresh } from "./actions.js";
import { renderChain, renderStats } from "./panels/chain/panel.js";
import { installNetwork, renderFooter, renderPeers } from "./panels/network/panel.js";
import { renderProducer } from "./panels/producing/panel.js";
import { installSending, renderPending } from "./panels/sending/panel.js";
import { installWallets, renderWallets } from "./panels/wallets/panel.js";

const POLL_MS = 4000;

wire({ node: NodeHttpClient, walletsRepository: WalletsLocalStorageRepository });

async function refresh() {
  try {
    await refreshView();
    renderStats();
    renderPeers();
    renderFooter();
    renderWallets();
    renderProducer();
    renderPending();
    renderChain();
  } catch (err) {
    toast(err.message, { error: true });
  }
}

onRefresh(refresh);
installWallets();
installSending();
installNetwork();

// Only the node's status is polled; blocks are fetched when its tip moves.
setInterval(async () => {
  if (document.hidden || isWorking()) return;
  try {
    if (await hasMoved()) return refresh();
    renderPeers();
  } catch { /* node down; the next tick tries again */ }
}, POLL_MS);

refresh();
