// The node page, put together: the adapters, the wallets and the view of the node are made here
// and nowhere else, then each panel is installed with them and draws itself whenever the view
// changes, and the node is polled for changes made elsewhere: other tabs, other nodes syncing,
// `POST /chain`.

import { NodeView } from "@application/node_view";
import { Wallets } from "@domain/wallets/wallets";
import { NodeHttpClient } from "@infra/node_client";
import { WalletsLocalStorageRepository } from "@infra/wallets_local_storage";
import { showError } from "@ui/dom";
import { Actions } from "./actions.ts";
import type { NodePage } from "./page.ts";
import { installChain } from "@ui/node/panels/chain/panel";
import { installNetwork } from "@ui/node/panels/network/panel";
import { installProducing } from "@ui/node/panels/producing/panel";
import { installSending } from "@ui/node/panels/sending/panel";
import { installWallets } from "@ui/node/panels/wallets/panel";

const POLL_MS = 4000;

const node = NodeHttpClient;
const wallets = new Wallets(WalletsLocalStorageRepository).load();
const view = new NodeView(node, wallets);
const refresh = (): Promise<void> => view.refresh().catch(showError);
const page: NodePage = {
  node,
  wallets,
  view,
  actions: new Actions(refresh, wallets),
};

installChain(page);
installWallets(page);
installSending(page);
installProducing(page);
installNetwork(page);

// Only the node's status is polled; blocks are fetched when its tip moves.
setInterval(async () => {
  if (document.hidden || page.actions.working) return;
  try {
    if (await view.hasMoved()) await refresh();
  } catch {
    /* node down; the next tick tries again */
  }
}, POLL_MS);

void refresh();
