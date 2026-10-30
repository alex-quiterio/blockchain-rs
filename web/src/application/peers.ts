// The node's place among its peers: adding one, and pulling from every one now.

import type { NodePort, SyncReport } from "./ports.ts";

export const addPeer = (
  node: NodePort,
  url: string,
): Promise<{ url: string; added: boolean }> => node.addPeer(url);
export const syncNow = (node: NodePort): Promise<SyncReport> => node.sync();
