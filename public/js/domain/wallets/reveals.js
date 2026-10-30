// A wallet's reveal chain, which it needs to stake and forge: a hash chain committed to once when
// staking, then revealed a link per forged block. A domain service: everything is worked out
// from the wallet's key, so nothing extra is stored.

import { sha256Hex } from "../encoding.js";

export const LENGTH = 1000; // the node's reveal chain length; both must agree

async function hashTimes(value, times) {
  for (let i = 0; i < times; i++) value = await sha256Hex(value);
  return value;
}

/** The secret start of commitment number `epoch`. */
const seed = (secretKey, epoch) => sha256Hex(`blockchain-rs/randao|${secretKey}|${epoch}`);

/** What a first stake publishes: the seed hashed LENGTH times. */
export async function commitment(secretKey, epoch) {
  return hashTimes(await seed(secretKey, epoch), LENGTH);
}

/** The link to reveal next, with `remaining` reveals left: one hash short of the last one shown. */
export async function reveal(secretKey, epoch, remaining) {
  return hashTimes(await seed(secretKey, epoch), remaining - 1);
}
