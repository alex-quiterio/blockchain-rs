// A transaction's page: what it is, who signed it, what it changes, and, once confirmed, the
// Merkle proof that it's in its block, each hash redone here.

import { auditTransaction, type InclusionAudit } from "@application/audit";
import { kindOf, signingPayload } from "@domain/transfers";
import { ago, el, fmt, short } from "@ui/dom";
import {
  checkRow,
  factRow,
  formula,
  href,
  link,
  panel,
  who,
  type Verdict,
} from "@ui/explorer/components";

export async function txPage(signature: string): Promise<HTMLElement[]> {
  const audit = await auditTransaction(signature);
  const { transaction: tx, placed, block } = audit;
  const kind = kindOf(tx);

  const summary = el("dl", { className: "facts" });
  factRow(
    summary,
    "Status",
    placed
      ? el("span", {
          className: "ok",
          textContent: `confirmed, ${fmt(placed.confirmations)} confirmation${placed.confirmations === 1 ? "" : "s"}`,
        })
      : el("span", {
          className: "pending",
          textContent: "pending: waiting in the mempool for a block",
        }),
  );
  if (placed && block) {
    factRow(
      summary,
      "Block",
      link(href.block(placed.height), `#${placed.height}`),
      ` · position ${placed.position} of ${block.transactions.length} · ${ago(placed.timestamp)}`,
    );
  }
  factRow(
    summary,
    "Kind",
    el("span", { className: `tag ${kind}`, textContent: kind }),
  );
  factRow(summary, "From", who(tx.from));
  factRow(summary, "To", who(tx.to));
  factRow(
    summary,
    "Amount",
    el("b", { textContent: fmt(tx.amount) }),
    tx.fee ? ` + ${fmt(tx.fee)} fee` : " · no fee",
  );
  factRow(
    summary,
    "Nonce",
    fmt(tx.nonce),
    ` · the sender's transaction number ${fmt(tx.nonce + 1)}. Each nonce is used once, which stops replays.`,
  );
  if (tx.commitment)
    factRow(
      summary,
      "Commitment",
      el("span", { className: "mono wrap", textContent: tx.commitment }),
      " · the end of the staker's reveal chain",
    );

  const signed = el("dl", { className: "facts" });
  factRow(
    signed,
    "Payload",
    el("span", { className: "formula wrap", textContent: signingPayload(tx) }),
  );
  factRow(
    signed,
    "Signature",
    el("span", { className: "mono wrap", textContent: tx.signature }),
  );
  checkRow(
    signed,
    "Verified",
    audit.signature.then((ok): Verdict => [
      ok,
      "ed25519, against the sender's address as public key",
    ]),
    "Change any field and this fails, which is why nobody can alter a transfer on its way.",
  );

  const effects = el("dl", { className: "facts" });
  const producer = block?.transactions[0]?.to;
  if (kind === "stake") {
    factRow(
      effects,
      "Sender",
      `−${fmt(tx.amount + (tx.fee || 0))} spendable, +${fmt(tx.amount)} staked`,
    );
  } else if (kind === "unstake") {
    factRow(
      effects,
      "Sender",
      `+${fmt(tx.amount)} spendable, −${fmt(tx.amount)} staked${tx.fee ? `, −${fmt(tx.fee)} fee` : ""}`,
    );
  } else {
    factRow(
      effects,
      "Sender",
      `−${fmt(tx.amount + (tx.fee || 0))}`,
      " ",
      who(tx.from),
    );
    factRow(effects, "Recipient", `+${fmt(tx.amount)}`, " ", who(tx.to));
  }
  if (tx.fee)
    factRow(
      effects,
      "Fee",
      `+${fmt(tx.fee)} to `,
      producer ? who(producer) : "whoever adds the block",
    );

  const views = [
    el(
      "h1",
      {},
      "Transaction ",
      el("span", { className: "mono muted", textContent: short(tx.signature) }),
    ),
    panel("What it is", summary),
    panel("Who signed it", signed),
    panel("What it changes", effects),
  ];
  if (audit.inclusion) {
    views.push(
      panel("Proof it's in the block", ...proofView(await audit.inclusion)),
    );
  }
  return views;
}

/** The Merkle path from this transaction up to its block's root, each hash redone here. */
function proofView(inclusion: InclusionAudit): HTMLElement[] {
  const { path, steps: proofSteps, height } = inclusion;
  const steps = el(
    "ol",
    { className: "path" },
    el(
      "li",
      {},
      el("span", { className: "mono", textContent: short(path[0] ?? "") }),
      " leaf: ",
      formula("sha256(leaf | the transaction's JSON)"),
    ),
    proofSteps.map((step, i) =>
      el(
        "li",
        {},
        el("span", {
          className: "mono",
          textContent: short(path[i + 1] ?? ""),
        }),
        " ",
        formula(
          step.side === "left"
            ? `sha256(node | ${short(step.hash)} | ↑)`
            : `sha256(node | ↑ | ${short(step.hash)})`,
        ),
        el("span", {
          className: "muted",
          textContent: ` sibling on the ${step.side}`,
        }),
      ),
    ),
  );

  const facts = el("dl", { className: "facts" });
  checkRow(
    facts,
    "Leaf",
    Promise.resolve<Verdict>([
      inclusion.leafMatches,
      "hashed here and matches the node's",
    ]),
  );
  checkRow(
    facts,
    "Root",
    Promise.resolve<Verdict>([
      inclusion.reachesRoot,
      `${proofSteps.length} hashes lead to the block's Merkle root`,
    ]),
    el("span", { className: "mono", textContent: short(inclusion.root) }),
  );
  checkRow(
    facts,
    "Header",
    inclusion.header.then((ok): Verdict => [
      ok,
      `its hash covers that root and is block #${height}'s`,
    ]),
  );
  factRow(
    facts,
    "Why it matters",
    `A light client keeps only headers (${fmt(height + 1)} so far) and checks this proof instead of downloading every transaction in the block.`,
  );
  return [facts, steps];
}
