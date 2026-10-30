// Drives both pages in headless Chrome, through the DevTools protocol, against a running node:
// the passphrase, wallets, mining or forging, sending, staking, locking, reloading, and every
// explorer page with its checks. Returns each step and any error the pages threw or logged.
// web/e2e/run.ts starts the nodes and runs it in both consensus modes.

import { spawn } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// The functions handed to `page()` and `until()` run inside the page, not here, and look its
// elements up loosely (querySelector<any>): a test that finds the wrong element fails on its own.

export interface Step {
  name: string;
  ok: boolean;
  detail: string;
}

export interface Outcome {
  steps: Step[];
  errors: string[];
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

export async function drive(
  chrome: string,
  base: string,
  mode: "pow" | "pos",
  port: number,
): Promise<Outcome> {
  const results: Step[] = [];
  const errors: string[] = [];
  const step = (name: string, ok: unknown, detail = "") =>
    results.push({ name, ok: Boolean(ok), detail: String(detail) });

  const browser = spawn(
    chrome,
    [
      "--headless=new",
      `--remote-debugging-port=${port}`,
      `--user-data-dir=${mkdtempSync(join(tmpdir(), "blockchain-rs-e2e-"))}`,
      "--no-first-run",
      // Ubuntu's AppArmor keeps Chrome's sandbox from starting on CI runners; locally it stays on.
      ...(process.env.CI ? ["--no-sandbox"] : []),
      "--no-default-browser-check",
      "about:blank",
    ],
    { stdio: "ignore" },
  );

  let ws: WebSocket | null = null;
  let id = 0;
  const pending = new Map<
    number,
    { resolve: (v: any) => void; reject: (e: Error) => void }
  >();
  const send = (method: string, params: object = {}): Promise<any> =>
    new Promise((resolve, reject) => {
      const n = ++id;
      pending.set(n, { resolve, reject });
      ws!.send(JSON.stringify({ id: n, method, params }));
    });
  /** Runs `fn` in the page and returns its (awaited, JSON) result. */
  async function page(
    fn: (...args: any[]) => unknown,
    ...args: unknown[]
  ): Promise<any> {
    const expression = `(${fn})(...${JSON.stringify(args)})`;
    const r = await send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
    });
    if (r.exceptionDetails)
      throw new Error(
        r.exceptionDetails.exception?.description || r.exceptionDetails.text,
      );
    return r.result.value;
  }
  /** Waits until `fn` in the page returns something truthy. */
  async function until(
    fn: (...args: any[]) => unknown,
    ms = 15000,
    ...args: unknown[]
  ): Promise<any> {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      try {
        const v = await page(fn, ...args);
        if (v) return v;
      } catch {
        /* page still loading */
      }
      await sleep(100);
    }
    return null;
  }
  async function go(url: string): Promise<void> {
    await send("Page.navigate", { url });
    await sleep(300);
  }

  try {
    for (let i = 0; i < 50 && !ws; i++) {
      try {
        const targets = (await (
          await fetch(`http://127.0.0.1:${port}/json/list`)
        ).json()) as { type: string; webSocketDebuggerUrl: string }[];
        const tab = targets.find((t) => t.type === "page")!;
        const socket = new WebSocket(tab.webSocketDebuggerUrl);
        await new Promise((resolve, reject) => {
          socket.onopen = resolve;
          socket.onerror = reject;
        });
        ws = socket;
      } catch {
        ws = null;
        await sleep(200);
      }
    }
    if (!ws) throw new Error("Chrome's DevTools never answered");
    ws.onmessage = ({ data }) => {
      const msg = JSON.parse(String(data));
      if (msg.id && pending.has(msg.id)) {
        const { resolve, reject } = pending.get(msg.id)!;
        pending.delete(msg.id);
        msg.error ? reject(new Error(msg.error.message)) : resolve(msg.result);
      } else if (msg.method === "Runtime.exceptionThrown") {
        errors.push(
          `exception: ${msg.params.exceptionDetails.exception?.description || msg.params.exceptionDetails.text}`,
        );
      } else if (
        msg.method === "Runtime.consoleAPICalled" &&
        ["error", "warning"].includes(msg.params.type)
      ) {
        errors.push(
          `console.${msg.params.type}: ${msg.params.args.map((a: any) => a.value ?? a.description).join(" ")}`,
        );
      } else if (
        msg.method === "Log.entryAdded" &&
        msg.params.entry.level === "error" &&
        !msg.params.entry.url?.endsWith("/favicon.ico")
      ) {
        errors.push(
          `log: ${msg.params.entry.text} ${msg.params.entry.url || ""}`,
        );
      }
    };
    await send("Page.enable");
    await send("Runtime.enable");
    await send("Log.enable");

    // ── The wallet page ───────────────────────────────────────────────
    await go(`${base}/`);
    step(
      "page loads and asks for a passphrase",
      await until(() =>
        document.querySelector<any>("#vault input[type=password]"),
      ),
    );
    step(
      "stats render",
      await until(
        () => document.querySelector<any>("#stat-height").textContent !== "–",
      ),
    );
    step(
      "chain shows valid",
      await until(() =>
        document
          .querySelector<any>("#validity")
          .textContent.includes("chain valid"),
      ),
    );

    await page(() => {
      const [a, b] = document.querySelectorAll<any>(
        "#vault input[type=password]",
      );
      a.value = b.value = "correct horse battery";
      a.closest("form").requestSubmit();
    });
    step(
      "passphrase set, vault unlocked",
      await until(
        () =>
          document
            .querySelector<any>("#vault")
            .textContent.includes("Unlocked"),
        20000,
      ),
    );

    for (const name of ["Alice", "Bob"]) {
      await page((n) => {
        document.querySelector<any>("#wallet-name").value = n;
        document.querySelector<any>("#wallet-form").requestSubmit();
      }, name);
      await until(
        (n) =>
          [...document.querySelectorAll<any>("#wallets .name")].some(
            (e) => e.textContent === n,
          ),
        10000,
        name,
      );
    }
    step(
      "two wallets created",
      (await page(
        () => document.querySelectorAll<any>("#wallets .wallet").length,
      )) === 2,
    );

    const produce = async () => {
      const before = await page(
        () => document.querySelector<any>("#stat-height").textContent,
      );
      await page(() =>
        document
          .querySelector<any>("#producer button:not([disabled])")
          ?.click(),
      );
      return until(
        (b) => document.querySelector<any>("#stat-height").textContent !== b,
        30000,
        before,
      );
    };
    if (mode === "pos") {
      step(
        "producer offers forging",
        await until(
          () =>
            document.querySelector<any>("#producer-title").textContent ===
            "Forge",
        ),
      );
    } else {
      step(
        "producer offers mining",
        await until(
          () =>
            document.querySelector<any>("#producer-title").textContent ===
            "Mine",
        ),
      );
    }
    step("first block added", await produce());
    step(
      "Alice is paid the reward",
      await until(
        () =>
          document.querySelector<any>("#wallets .balance").textContent !== "0",
      ),
    );

    // Send Bob 5 with a fee of 1.
    await page(() => {
      const [alice, bob] = [
        ...document.querySelectorAll<any>("#send-from option"),
      ].map((o) => o.value);
      document.querySelector<any>("#send-from").value = alice;
      document.querySelector<any>("#send-to").value = bob;
      document.querySelector<any>("#send-amount").value = "5";
      document.querySelector<any>("#send-fee").value = "1";
      document
        .querySelector<any>("#send-fee")
        .dispatchEvent(new Event("input"));
    });
    step(
      "send summary explains the cost",
      (
        await page(
          () => document.querySelector<any>("#send-summary").textContent,
        )
      ).includes("1 fee"),
    );
    await page(() => document.querySelector<any>("#send-form").requestSubmit());
    step(
      "transfer waits in the mempool",
      await until(
        () => document.querySelector<any>("#stat-pending").textContent === "1",
      ),
    );
    step("second block takes it", await produce());
    step(
      "Bob received 5",
      await until(() =>
        [...document.querySelectorAll<any>("#wallets .wallet")].some(
          (w) =>
            w.querySelector(".name").textContent === "Bob" &&
            w.querySelector(".balance").textContent === "5",
        ),
      ),
    );

    if (mode === "pos") {
      // Alice stakes, which commits her reveal chain; then she's drawn and must forge with a reveal.
      await page(() => {
        document.querySelector<any>("#send-from").value = [
          ...document.querySelectorAll<any>("#send-from option"),
        ][0].value;
        document.querySelector<any>("#send-to").value = "__stake";
        document.querySelector<any>("#send-amount").value = "10";
        document.querySelector<any>("#send-fee").value = "";
        document.querySelector<any>("#send-form").requestSubmit();
      });
      step(
        "stake waits in the mempool",
        await until(
          () =>
            document.querySelector<any>("#stat-pending").textContent === "1",
        ),
      );
      step("block with the stake forged", await produce());
      step(
        "Alice is now the validator",
        await until(() =>
          document
            .querySelector<any>("#producer")
            .textContent.includes("Alice's turn"),
        ),
      );
      step("Alice forges with her reveal", await produce());
      step(
        "stake bar shows",
        await until(() => document.querySelector<any>(".stakebar span")),
      );
    }

    // Lock wipes keys; unlocking brings them back.
    await page(() =>
      [...document.querySelectorAll<any>("#vault button")]
        .find((b) => b.textContent === "Lock")
        .click(),
    );
    step(
      "locking hides the wallet form",
      await until(() => document.querySelector<any>("#wallet-form").hidden),
    );
    await page(() => {
      const i = document.querySelector<any>("#unlock-passphrase");
      i.value = "correct horse battery";
      i.closest("form").requestSubmit();
    });
    step(
      "unlocking with the passphrase works",
      await until(
        () =>
          document
            .querySelector<any>("#vault")
            .textContent.includes("Unlocked"),
        20000,
      ),
    );

    // A block's breakdown redoes the Merkle root in the page.
    await page(() => {
      document.querySelector<any>(".breakdown").open = true;
    });
    step(
      "breakdown checks the Merkle root",
      await until(() =>
        document
          .querySelector<any>(".breakdown")
          .textContent.includes("match the Merkle root"),
      ),
    );
    step(
      "blocks show hashes and payouts",
      await page(
        () => document.querySelectorAll<any>("#blocks .block").length >= 3,
      ),
    );

    // Survives a reload: the vault persists, encrypted.
    await go(`${base}/`);
    step(
      "after reload the vault asks to unlock",
      await until(() => document.querySelector<any>("#unlock-passphrase")),
    );
    step(
      "wallets are still listed while locked",
      await until(
        () => document.querySelectorAll<any>("#wallets .wallet").length === 2,
      ),
    );

    // ── The explorer ──────────────────────────────────────────────────
    const sig = await page(async (b) => {
      const blocks = await (await fetch(`${b}/chain?from=2&limit=1`)).json();
      return blocks[0].transactions[1].signature;
    }, base);
    const bob = await page(
      () => [...document.querySelectorAll<any>("#send-to option")][1].value,
    );

    await go(`${base}/explorer.html`);
    step(
      "explorer home lists the latest blocks",
      await until(
        () =>
          document.querySelectorAll<any>("#view table.grid tbody tr").length >=
          3,
      ),
    );

    await go(`${base}/explorer.html#/block/2`);
    step(
      "block page loads",
      await until(() =>
        document
          .querySelector<any>("#view h1")
          ?.textContent.startsWith("Block #2"),
      ),
    );
    step(
      "block page: every check passes",
      await until(
        () =>
          document.querySelectorAll<any>("#view .ok").length >= 4 &&
          !/✗|checking…/.test(document.querySelector<any>("#view").textContent),
      ),
    );

    await go(`${base}/explorer.html#/tx/${sig}`);
    step(
      "transaction page loads",
      await until(() =>
        document
          .querySelector<any>("#view h1")
          ?.textContent.startsWith("Transaction"),
      ),
    );
    step(
      "transaction page: signature, leaf, root and header check out",
      await until(
        () =>
          document.querySelectorAll<any>("#view .ok").length >= 5 &&
          !/✗|checking…/.test(document.querySelector<any>("#view").textContent),
      ),
    );

    await go(`${base}/explorer.html#/address/${bob}`);
    step(
      "address page loads",
      await until(() =>
        document
          .querySelector<any>("#view h1")
          ?.textContent.startsWith("Address"),
      ),
    );
    step(
      "address page: history adds up",
      await until(() =>
        document
          .querySelector<any>("#view")
          .textContent.includes("✓ in − out − stake = balance"),
      ),
    );

    await page(() => {
      document.querySelector<any>("#query").value = "1";
      document.querySelector<any>("#search").requestSubmit();
    });
    step(
      "search by height finds the block",
      await until(
        () =>
          location.hash === "#/block/1" &&
          document
            .querySelector<any>("#view h1")
            ?.textContent.startsWith("Block #1"),
      ),
    );
    await page(() => {
      document.querySelector<any>("#query").value = "nonsense";
      document.querySelector<any>("#search").requestSubmit();
    });
    step(
      "a bad search says what to try",
      await until(() =>
        document
          .querySelector<any>("#toasts")
          .textContent.includes("Try a block height"),
      ),
    );
  } catch (e) {
    step("test harness", false, e instanceof Error ? e.stack : String(e));
  } finally {
    browser.kill();
  }
  return { steps: results, errors };
}
