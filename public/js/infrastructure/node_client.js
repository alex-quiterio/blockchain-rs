// The node, over its HTTP API: an adapter for the port the application talks to the node through.
// Each method is one request, named for what it asks; a refusal comes back as an Error carrying
// the node's own message.

async function request(method, path, body) {
  const res = await fetch(path, {
    method,
    headers: body ? { "content-type": "application/json" } : {},
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => null);
  if (!res.ok) throw new Error(data?.message || `${res.status} ${res.statusText}`);
  return data;
}

const id = encodeURIComponent;

export const NodeHttpClient = {
  status: () => request("GET", "/status"),
  info: () => request("GET", "/node"),
  consensus: () => request("GET", "/consensus"),
  validity: () => request("GET", "/chain/valid"),
  blocks: (from, limit) => request("GET", `/chain?from=${from}${limit != null ? `&limit=${limit}` : ""}`),
  works: (from, limit) => request("GET", `/chain/work?from=${from}&limit=${limit}`),
  block: (heightOrHash) => request("GET", `/blocks/${id(heightOrHash)}`),
  pending: () => request("GET", "/transactions/pending"),
  fees: () => request("GET", "/fees"),
  transaction: (signature) => request("GET", `/transactions/${id(signature)}`),
  proof: (signature) => request("GET", `/transactions/${id(signature)}/proof`),
  balance: (address) => request("GET", `/balances/${id(address)}`),
  history: (address) => request("GET", `/addresses/${id(address)}/transactions`),
  peers: () => request("GET", "/peers"),

  submitTransaction: (tx) => request("POST", "/transactions", tx),
  mine: (miner) => request("POST", "/mine", { miner }),
  propose: (validator, reveal) => request("POST", "/forge/proposal", { validator, reveal }),
  submitBlock: (block) => request("POST", "/blocks", block),
  addPeer: (url) => request("POST", "/peers", { url }),
  sync: () => request("POST", "/sync"),
};
