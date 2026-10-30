// The node, over its HTTP API: an adapter for the application's NodePort. Each method is one
// request, named for what it asks; a refusal comes back as an Error carrying the node's own
// message. What it sends is checked against the node's own request bodies, and what the port
// says comes back against its responses, both generated from the Rust code into node_wire.ts.

import type {
  Account,
  Consensus,
  Fees,
  Located,
  NodeInfo,
  NodePort,
  Peer,
  Placement,
  Proof,
  Status,
  SyncReport,
  Validity,
} from "@application/ports";
import type { Block, BlockHeader, ProofStep } from "@domain/blocks";
import type { Transfer } from "@domain/transfers";
import type * as wire from "./node_wire.ts";

/** True only when A and B are the same type, optional and nullable fields included. */
type Equals<A, B> =
  (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2
    ? true
    : false;
type Assert<T extends true> = T;

/**
 * The application's names for what the node sends, each the same type as the node's own. If a
 * body changes in Rust, regenerating node_wire.ts makes the matching line here fail to compile.
 */
export type MatchesTheNode = [
  Assert<Equals<Status, wire.StatusResponse>>,
  Assert<Equals<NodeInfo, wire.NodeResponse>>,
  Assert<Equals<Validity, wire.ValidityResponse>>,
  Assert<Equals<Consensus, wire.ConsensusResponse>>,
  Assert<Equals<Account, wire.BalanceResponse>>,
  Assert<Equals<Fees, wire.FeesResponse>>,
  Assert<Equals<Placement, wire.PlacementResponse>>,
  Assert<Equals<Located, wire.TransactionResponse>>,
  Assert<Equals<Proof, wire.ProofResponse>>,
  Assert<Equals<Peer, wire.PeerResponse>>,
  Assert<Equals<SyncReport, wire.SyncResponse>>,
  Assert<Equals<Block, wire.Block>>,
  Assert<Equals<BlockHeader, wire.BlockHeader>>,
  Assert<Equals<ProofStep, wire.Step>>,
  Assert<Equals<Transfer, wire.Transaction>>,
  Assert<
    Equals<Awaited<ReturnType<NodePort["addPeer"]>>, wire.AddPeerResponse>
  >,
];

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
  const data: Partial<wire.ErrorResponse> | null = await res
    .json()
    .catch(() => null);
  if (!res.ok)
    throw new Error(data?.message || `${res.status} ${res.statusText}`);
  return data as T;
}

const id = (value: string | number): string => encodeURIComponent(value);

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

  submitTransaction: (tx) =>
    request("POST", "/transactions", tx satisfies wire.Transaction),
  mine: (miner) =>
    request("POST", "/mine", { miner } satisfies wire.MineRequest),
  propose: (validator, reveal) =>
    request("POST", "/forge/proposal", {
      validator,
      reveal,
    } satisfies wire.ProposalRequest),
  submitBlock: (block) =>
    request("POST", "/blocks", block satisfies wire.Block),
  addPeer: (url) =>
    request("POST", "/peers", { url } satisfies wire.PeerRequest),
  sync: () => request("POST", "/sync"),
};
