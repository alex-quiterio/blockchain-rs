// The node page, put together: the adapters are wired into the application here and nowhere
// else, then each panel is installed and drawn, and the node is polled for changes made
// elsewhere: other tabs, other nodes syncing, `POST /chain`.

import { hasMoved, refreshView } from "@application/node_view";
import { wire } from "@application/ports";
import { NodeHttpClient } from "@infra/node_client";
import { WalletsLocalStorageRepository } from "@infra/repository/local_storage";
import { toast } from "@ui/dom";
import { isWorking, onRefresh } from "./actions.ts";
import { renderChain, renderStats } from "@ui/node/panels/chain/panel";
import {
  installNetwork,
  renderFooter,
  renderPeers,
} from "@ui/node/panels/network/panel";
import { renderProducer } from "@ui/node/panels/producing/panel";
import { installSending, renderPending } from "@ui/node/panels/sending/panel";
import { installWallets, renderWallets } from "@ui/node/panels/wallets/panel";

const POLL_MS = 4000;

wire({
  node: NodeHttpClient,
  walletsRepository: WalletsLocalStorageRepository,
});

async function refresh(): Promise<void> {
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
    toast(err instanceof Error ? err.message : String(err), { error: true });
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
  } catch {
    /* node down; the next tick tries again */
  }
}, POLL_MS);

void refresh();
