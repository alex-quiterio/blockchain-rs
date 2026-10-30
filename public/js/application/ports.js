// What the application talks through, wired in by each page's main.js, so the use cases never
// name an adapter: `node`, the node's API, and `walletsRepository`, where the wallets are kept.

export const ports = { node: null, walletsRepository: null };

export function wire(adapters) {
  Object.assign(ports, adapters);
}
