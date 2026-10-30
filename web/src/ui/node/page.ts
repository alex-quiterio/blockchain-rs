// What the node page's panels are built with: the wallets, the view of the node they draw from,
// what their buttons can ask for, and how those buttons run. main.ts puts it together, once.
// Panels never hold the node's port, so whatever they do goes through a use case.

import type { NodeView } from "@application/node_view";
import type { SyncReport } from "@application/ports";
import type { Sending } from "@application/sending";
import type { Block } from "@domain/blocks";
import type { Transfer } from "@domain/transfers";
import type { Wallet, Wallets } from "@domain/wallets/wallets";
import type { Actions } from "./actions.ts";

/** The use cases the panels' buttons run, already given the node and the wallets. */
export interface Commands {
  createWallet(name: string): Promise<string>;
  send(sending: Sending): Promise<Transfer>;
  mine(miner: string): Promise<Block>;
  forge(wallet: Wallet, validatorDrawn: boolean): Promise<Block>;
  addPeer(url: string): Promise<{ url: string; added: boolean }>;
  syncNow(): Promise<SyncReport>;
}

export interface NodePage {
  wallets: Wallets;
  view: NodeView;
  commands: Commands;
  actions: Actions;
}
