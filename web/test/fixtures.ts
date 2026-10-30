// The fixtures web/scripts/record_fixtures.ts recorded from live nodes, typed.

import { readFileSync } from "node:fs";
import type { Proof } from "@application/ports";
import type { Block } from "@domain/blocks";

const read = <T>(name: string): T =>
  JSON.parse(
    readFileSync(new URL(`./fixtures/${name}`, import.meta.url), "utf8"),
  ) as T;

/** A mined block holding a payout and three transfers, one with a fee, and the work it needed. */
export const mined = read<{ block: Block; work: number }>("mined_block.json");
/** The node's proof that the mined block's last transfer is in it. */
export const proof = read<Proof>("proof.json");
/** A block forged by a drawn validator: it carries their reveal and their signature over its hash. */
export const forged = read<Block>("forged_block.json");
