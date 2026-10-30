// The node's place among its peers: adding one, and pulling from every one now.

import { ports, type SyncReport } from "./ports.ts";

export const addPeer = (
  url: string,
): Promise<{ url: string; added: boolean }> => ports.node.addPeer(url);
export const syncNow = (): Promise<SyncReport> => ports.node.sync();
