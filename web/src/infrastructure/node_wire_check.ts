// Holds the HTTP adapter to the node's own bodies, at compile time only. node_wire.ts is
// generated from the Rust code by `cargo test`, never committed, and left out of the build, so
// only `npm run check` reads this file, after regenerating it. If a body changes in Rust, the
// matching line here stops compiling until the UI's type follows.

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
import type { MineBody, PeerBody, ProposalBody } from "./node_client.ts";
import type * as wire from "./node_wire.ts";

/** True only when A and B are the same type, optional and nullable fields included. */
type Equals<A, B> =
  (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2
    ? true
    : false;
/** True when every A is a B: what the client sends must be something the node accepts. */
type Accepts<B, A> = [A] extends [B] ? true : false;
type Assert<T extends true> = T;

/** Each type NodePort answers with is the node's own, under the application's name for it. */
export type Answers = [
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

/** Each body the client sends is one the node takes. Transfers and blocks are checked above. */
export type Requests = [
  Assert<Accepts<wire.MineRequest, MineBody>>,
  Assert<Accepts<wire.ProposalRequest, ProposalBody>>,
  Assert<Accepts<wire.PeerRequest, PeerBody>>,
];
