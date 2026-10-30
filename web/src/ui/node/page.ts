// What the node page's panels are built with: the node's port, the wallets, the view of the node
// they draw from, and the actions their buttons run. main.ts puts it together, once.

import type { NodeView } from "@application/node_view";
import type { NodePort } from "@application/ports";
import type { Wallets } from "@domain/wallets/wallets";
import type { Actions } from "./actions.ts";

export interface NodePage {
  node: NodePort;
  wallets: Wallets;
  view: NodeView;
  actions: Actions;
}
