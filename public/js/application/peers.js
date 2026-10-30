// The node's place among its peers: adding one, and pulling from every one now.

import { ports } from "./ports.js";

export const addPeer = (url) => ports.node.addPeer(url);
export const syncNow = () => ports.node.sync();
