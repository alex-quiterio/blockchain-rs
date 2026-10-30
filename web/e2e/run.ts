// The browser test, end to end: builds nothing itself, but starts a fresh node from the release
// binary in each consensus mode and drives both pages against it in headless Chrome.
//   cargo build --release && npm run build && npm run e2e
// Chrome is looked for in the usual places, or wherever CHROME says.

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { drive } from "./browser.ts";

const NODE_BINARY = new URL("../../target/release/blockchain", import.meta.url)
  .pathname;
const UI = new URL("../../public/js/ui/node/main.js", import.meta.url).pathname;
const CHROMES = [
  process.env.CHROME,
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/usr/bin/google-chrome",
  "/usr/bin/chromium",
  "/usr/bin/chromium-browser",
].filter((c): c is string => Boolean(c));

const chrome = CHROMES.find(existsSync);
const missing = [
  !chrome && "Chrome (set CHROME to its path)",
  !existsSync(NODE_BINARY) && "the node (cargo build --release)",
  !existsSync(UI) && "the UI (npm run build)",
].filter(Boolean);
if (missing.length) {
  console.error(`The browser test needs ${missing.join(", ")}.`);
  process.exit(2);
}

let failures = 0;
for (const [i, mode] of (["pow", "pos"] as const).entries()) {
  const port = 3980 + i;
  const node = spawn(NODE_BINARY, [], {
    stdio: "ignore",
    env: {
      ...process.env,
      CONSENSUS: mode,
      DIFFICULTY: "2",
      TARGET_BLOCK_SECS: "0",
      BIND_ADDR: `127.0.0.1:${port}`,
      CHAIN_FILE: join(
        mkdtempSync(join(tmpdir(), "blockchain-rs-e2e-")),
        "chain.json",
      ),
    },
  });
  const base = `http://127.0.0.1:${port}`;
  try {
    for (let tries = 0; tries < 50; tries++) {
      if (
        await fetch(`${base}/status`).then(
          (r) => r.ok,
          () => false,
        )
      )
        break;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    const { steps, errors } = await drive(chrome!, base, mode, 9330 + i);
    const failed = steps.filter((s) => !s.ok);
    console.log(
      `${mode === "pow" ? "proof of work" : "proof of stake"}: ${steps.length - failed.length} of ${steps.length} steps passed`,
    );
    for (const s of failed)
      console.log(
        `  ✗ ${s.name}${s.detail ? `: ${s.detail.slice(0, 300)}` : ""}`,
      );
    for (const e of errors) console.log(`  ! ${e.slice(0, 300)}`);
    failures += failed.length + errors.length;
  } finally {
    node.kill();
  }
}
process.exit(failures ? 1 : 0);
