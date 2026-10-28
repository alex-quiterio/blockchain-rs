// Shared by the wallet page (app.js) and the block explorer (explorer.js): talking to the node,
// building views, and redoing the node's checks in the browser.

const $ = (id) => document.getElementById(id);
const COINBASE = "coinbase";
const STAKE = "stake";
const UNSTAKE = "unstake";

// ---- Node API ---------------------------------------------------------------

async function api(method, path, body) {
  const res = await fetch(path, {
    method,
    headers: body ? { "content-type": "application/json" } : {},
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => null);
  if (!res.ok) throw new Error(data?.message || `${res.status} ${res.statusText}`);
  return data;
}

// ---- Bytes and hashes -------------------------------------------------------

const toHex = (bytes) => [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
const fromHex = (hex) => new Uint8Array(hex.match(/../g).map((h) => parseInt(h, 16)));

async function sha256(text) {
  return toHex(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text))));
}

const kindOf = (tx) => (tx.from === COINBASE ? "coinbase" : tx.to === STAKE ? "stake" : tx.to === UNSTAKE ? "unstake" : "transfer");
const feesOf = (txs) => txs.reduce((sum, tx) => sum + (tx.fee || 0), 0);

/** Merkle roots, computed exactly as the node does (see domain/merkle.rs). */
const Merkle = {
  EMPTY_ROOT: "0".repeat(64),

  leaf(tx) {
    return sha256(`leaf|${JSON.stringify(tx)}`);
  },

  node(left, right) {
    return sha256(`node|${left}|${right}`);
  },

  async root(transactions) {
    let level = await Promise.all(transactions.map((tx) => this.leaf(tx)));
    if (!level.length) return this.EMPTY_ROOT;
    while (level.length > 1) {
      const pairs = [];
      for (let i = 0; i < level.length; i += 2) {
        // An odd hash out moves up as it is.
        pairs.push(i + 1 < level.length ? this.node(level[i], level[i + 1]) : level[i]);
      }
      level = await Promise.all(pairs);
    }
    return level[0];
  },
};

/** The node's checks, redone here so nothing has to be taken on trust. */
const Check = {
  /** The timestamp as the node hashes it: always nine fractional digits. Its JSON trims them. */
  timestamp(iso) {
    const [, whole, fraction = ""] = iso.match(/^(.*T\d\d:\d\d:\d\d)(?:\.(\d+))?Z$/);
    return `${whole}.${fraction.padEnd(9, "0")}Z`;
  },

  /** `index|timestamp|prev_hash|merkle_root|nonce`, then the validator and reveal if present. */
  headerHash(block) {
    const extras = [block.validator, block.reveal].filter(Boolean).map((x) => `|${x}`).join("");
    return sha256(`${block.index}|${this.timestamp(block.timestamp)}|${block.prev_hash}|${block.merkle_root}|${block.nonce}${extras}`);
  },

  /** What the sender signed: `from|to|amount|nonce`, then `|fee=N` and `|commit=C` if present. */
  signingPayload(tx) {
    const extras = `${tx.fee ? `|fee=${tx.fee}` : ""}${tx.commitment ? `|commit=${tx.commitment}` : ""}`;
    return `${tx.from}|${tx.to}|${tx.amount}|${tx.nonce}${extras}`;
  },

  /** Whether `signature` is `address`'s ed25519 signature over `message`. */
  async signedBy(address, message, signature) {
    try {
      const key = await crypto.subtle.importKey("raw", fromHex(address), { name: "Ed25519" }, false, ["verify"]);
      return await crypto.subtle.verify({ name: "Ed25519" }, key, fromHex(signature), new TextEncoder().encode(message));
    } catch {
      return false;
    }
  },

  /** Proof of work: the hash's first 128 bits are at most 2^128 / work. */
  meetsWork(hash, work) {
    return BigInt(`0x${hash.slice(0, 32)}`) <= ((1n << 128n) - 1n) / BigInt(work);
  },

  /** Follows a Merkle proof from a leaf; returns every hash on the way up, the root last. */
  async climb(leaf, steps) {
    const path = [leaf];
    for (const step of steps) {
      const here = path.at(-1);
      path.push(await (step.side === "left" ? Merkle.node(step.hash, here) : Merkle.node(here, step.hash)));
    }
    return path;
  },
};

// ---- View helpers -----------------------------------------------------------

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  Object.assign(node, attrs);
  node.append(...children.flat().filter((c) => c != null && c !== false));
  return node;
}

const short = (hex) => (hex.length > 16 ? `${hex.slice(0, 8)}…${hex.slice(-6)}` : hex);
const fmt = (n) => Number(n).toLocaleString();

const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
function ago(timestamp) {
  const s = Math.round((new Date(timestamp) - Date.now()) / 1000);
  for (const [unit, size] of [["day", 86400], ["hour", 3600], ["minute", 60]]) {
    if (Math.abs(s) >= size) return rtf.format(Math.round(s / size), unit);
  }
  return Math.abs(s) < 5 ? "just now" : rtf.format(s, "second");
}
