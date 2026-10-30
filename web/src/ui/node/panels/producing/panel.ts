// The producer panel: mining a block under proof of work, or forging one under proof of stake,
// with whose turn it is and how the stake is split.

import { forge, mine } from "@application/producing";
import type { Wallet } from "@domain/wallets/wallets";
import { $, el, explain, fmt, toast } from "@ui/dom";
import type { NodePage } from "@ui/node/page";
import {
  avatar,
  fillWalletSelect,
  hues,
  nameOf,
  party,
} from "@ui/node/parties";
import { fmtWork } from "@ui/node/work";

const pct = (part: number, whole: number): string =>
  `${whole ? Math.round((part / whole) * 100) : 0}%`;

/** Draws mining or forging, whichever the node's rules call for, each time the view changes. */
export function installProducing(page: NodePage): void {
  page.view.onChange(() => renderProducer(page));
}

function renderProducer(page: NodePage): void {
  if (page.view.isStake()) return renderForge(page);
  $("producer-title").textContent = "Mine";
  const select = el("select", { id: "miner" });
  fillWalletSelect(select, page.wallets);
  const button = el("button", { className: "gold", textContent: "Mine block" });
  const form = el(
    "form",
    { className: "inline" },
    el("label", { className: "grow" }, "Reward to ", select),
    button,
  );
  form.onsubmit = (e) => {
    e.preventDefault();
    mineAs(page, button, select.value);
  };
  const c = page.view.consensus();
  const work = Number(c.work);
  const pace = c.target_block_secs
    ? el(
        "p",
        { className: "summary pace" },
        "Next block: about ",
        el("b", { textContent: fmtWork(work) }),
        " hashes. Aiming for one block every ",
        el("b", { textContent: `${c.target_block_secs}s` }),
        `; the next adjustment is at block ${fmt(c.next_retarget ?? 0)}.`,
      )
    : el(
        "p",
        { className: "summary pace" },
        "Next block: about ",
        el("b", { textContent: fmtWork(work) }),
        " hashes, at a fixed difficulty.",
      );
  $("producer").replaceChildren(
    form,
    pace,
    explain(
      "How does proof of work work?",
      `The miner bundles the mempool and a payout to themselves into a block, then hashes it again and again, changing the <code>nonce</code> each time.`,
      `The block counts only once its hash, read as a number, is below a target. Right now that takes about ${fmtWork(work)} tries, so rewriting history means redoing all that work.`,
      c.target_block_secs
        ? `The target moves to keep blocks coming every ${c.target_block_secs}s. Every ${c.retarget_interval} blocks the node checks how long they took: faster means harder, slower means easier, by at most 4× at a time. Timestamps must be after the median of the last 11 and at most 2 hours ahead, so miners can't fake the pace.`
        : `The difficulty is fixed on this node. Start it with <code>TARGET_BLOCK_SECS</code> above 0 to let it adjust.`,
      `When forks disagree, the chain with the most total work wins, not the longest. Exact ties go to the lower tip hash, so every node picks the same one.`,
    ),
  );
}

function renderForge(page: NodePage): void {
  const { wallets } = page;
  $("producer-title").textContent = "Forge";
  const {
    next_validator: next,
    stakes,
    total_stake: total,
  } = page.view.consensus();
  const ours = next ? wallets.byAddress(next) : undefined;
  const children: HTMLElement[] = [];

  if (!next) {
    const select = el("select");
    fillWalletSelect(select, wallets);
    const button = el("button", {
      className: "stake",
      textContent: "Forge block",
    });
    const form = el(
      "form",
      { className: "inline" },
      el("label", { className: "grow" }, "Forge as ", select),
      button,
    );
    form.onsubmit = (e) => {
      e.preventDefault();
      forgeAs(page, button, wallets.byAddress(select.value));
    };
    children.push(
      el(
        "div",
        { className: "turn" },
        el(
          "div",
          {},
          el("div", { className: "who", textContent: "Anyone's turn" }),
          el("div", {
            className: "sub",
            textContent:
              "Nobody has staked yet, so any wallet may forge. Forge a few blocks, then stake some coins.",
          }),
        ),
      ),
      form,
    );
  } else {
    const button = el("button", {
      className: "stake",
      textContent: ours ? `Forge as ${ours.name}` : "Not your turn",
      disabled: !ours,
    });
    button.onclick = () => forgeAs(page, button, ours);
    children.push(
      el(
        "div",
        { className: "turn" },
        avatar(next),
        el(
          "div",
          { className: "grow" },
          el("div", {
            className: "who",
            textContent: `${nameOf(wallets, next)}'s turn`,
          }),
          el("div", {
            className: "sub",
            textContent: ours
              ? "Their key is in this browser."
              : "Their key isn't in this browser, so only they can forge.",
          }),
        ),
      ),
      button,
    );
  }

  if (stakes.length) {
    children.push(
      el("h3", { textContent: `Stake · ${fmt(total)} locked` }),
      el(
        "div",
        { className: "stakebar" },
        stakes.map((s) => {
          const seg = el("span", {
            title: `${nameOf(wallets, s.address)}: ${fmt(s.amount)}`,
          });
          seg.style.width = pct(s.amount, total);
          const [from, to] = hues(s.address);
          seg.style.background = `linear-gradient(90deg, hsl(${from} 75% 58%), hsl(${to} 70% 42%))`;
          return seg;
        }),
      ),
      el(
        "ul",
        { className: "legend" },
        stakes.map((s) =>
          el(
            "li",
            { className: s.address === next ? "next" : "" },
            party(wallets, s.address),
            el("span", { className: "fee", textContent: fmt(s.amount) }),
            el("span", {
              className: "pct",
              textContent: `${pct(s.amount, total)} chance`,
            }),
          ),
        ),
      ),
    );
  }

  children.push(
    explain(
      "How does proof of stake work?",
      `Validators lock coins by sending them to <code>stake</code>. Locked coins can't be spent until they're unstaked.`,
      `For each block, one validator is drawn at random, weighted by stake, from a randomness <b>beacon</b>. Every node computes the same draw.`,
      `When you first stake, this page commits to the end of a secret hash chain made from your key. Each block you forge must reveal the next link back, which anyone can check by hashing it once.`,
      `Reveals are fixed the moment you commit, so a forger can't try different timestamps or contents to make the draw pick them again. Each reveal is mixed into the beacon, which picks the next validator.`,
      `That validator signs the block instead of doing work. Anyone can check the signature, and a block from the wrong validator is rejected.`,
      `The validator earns the reward plus the block's fees, so more stake means more turns and more income.`,
      `When forks disagree, the one with the most stake behind its blocks wins, not the longest. A long chain nobody staked on can't outweigh a short one backed by real stake.`,
    ),
  );
  $("producer").replaceChildren(...children);
}

/** Shows that a block is being produced, with a running clock; the returned function stops it. */
function showProducing(text: string): (message?: string) => void {
  const status = $("producing");
  const started = performance.now();
  const tick = () =>
    status.replaceChildren(
      el("span", { className: "spin" }),
      el(
        "span",
        {},
        text,
        ` · ${((performance.now() - started) / 1000).toFixed(1)}s`,
      ),
    );
  tick();
  status.hidden = false;
  const timer = setInterval(tick, 100);
  return (message) => {
    clearInterval(timer);
    if (!message) {
      status.hidden = true;
      return;
    }
    status.replaceChildren(
      el(
        "span",
        {},
        message,
        ` in ${((performance.now() - started) / 1000).toFixed(2)}s`,
      ),
    );
    setTimeout(() => (status.hidden = true), 5000);
  };
}

function mineAs(
  { node, view, actions }: NodePage,
  button: HTMLButtonElement,
  miner: string,
): void {
  if (!miner) return toast("Create a wallet first", { error: true });
  const done = showProducing(
    `Trying about ${fmtWork(Number(view.consensus().work))} hashes`,
  );
  void actions.busy(button, async () => {
    try {
      const block = await mine(node, miner);
      done(`Block ${block.index} found on try ${fmt(block.nonce + 1)}`);
    } catch (err) {
      done();
      throw err;
    }
  });
}

function forgeAs(
  { node, wallets, view, actions }: NodePage,
  button: HTMLButtonElement,
  wallet: Wallet | undefined,
): void {
  if (!wallet) return toast("Create a wallet first", { error: true });
  void actions.busy(button, async () => {
    actions.requireUnlocked();
    const block = await forge(
      node,
      wallets,
      wallet,
      Boolean(view.consensus().next_validator),
    );
    toast(`${wallet.name} forged block ${block.index}`);
  });
}
