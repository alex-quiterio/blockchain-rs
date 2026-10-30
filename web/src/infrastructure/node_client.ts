// The node, over its HTTP API: an adapter for the application's NodePort. Each method is one
// request, named for what it asks; a refusal comes back as an Error carrying the node's own
// message.

import type { NodePort } from "@application/ports";

async function request<T>(
  method: "GET" | "POST",
  path: string,
  body?: unknown,
): Promise<T> {
  const res = await fetch(path, {
    method,
    headers: body ? { "content-type": "application/json" } : {},
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => null);
  if (!res.ok)
    throw new Error(data?.message || `${res.status} ${res.statusText}`);
  return data as T;
}

const id = (value: string | number): string => encodeURIComponent(value);

/** What the requests with a body of their own send. node_wire_check.ts holds them to the node's. */
export interface MineBody {
  miner: string;
}
export interface ProposalBody {
  validator: string;
  reveal: string | undefined;
}
export interface PeerBody {
  url: string;
}

export const NodeHttpClient: NodePort = {
  status: () => request("GET", "/status"),
  info: () => request("GET", "/node"),
  consensus: () => request("GET", "/consensus"),
  validity: () => request("GET", "/chain/valid"),
  blocks: (from, limit) =>
    request(
      "GET",
      `/chain?from=${from}${limit != null ? `&limit=${limit}` : ""}`,
    ),
  works: (from, limit) =>
    request("GET", `/chain/work?from=${from}&limit=${limit}`),
  block: (heightOrHash) => request("GET", `/blocks/${id(heightOrHash)}`),
  pending: () => request("GET", "/transactions/pending"),
  fees: () => request("GET", "/fees"),
  transaction: (signature) => request("GET", `/transactions/${id(signature)}`),
  proof: (signature) => request("GET", `/transactions/${id(signature)}/proof`),
  balance: (address) => request("GET", `/balances/${id(address)}`),
  history: (address) =>
    request("GET", `/addresses/${id(address)}/transactions`),
  peers: () => request("GET", "/peers"),

  submitTransaction: (tx) => request("POST", "/transactions", tx),
  mine: (miner) => request("POST", "/mine", { miner } satisfies MineBody),
  propose: (validator, reveal) =>
    request("POST", "/forge/proposal", {
      validator,
      reveal,
    } satisfies ProposalBody),
  submitBlock: (block) => request("POST", "/blocks", block),
  addPeer: (url) => request("POST", "/peers", { url } satisfies PeerBody),
  sync: () => request("POST", "/sync"),
};
