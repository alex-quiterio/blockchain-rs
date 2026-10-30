// Running an action from a button: one at a time, with the page refreshed after and any refusal
// shown. Panels use it; main.js says how the page refreshes.

import { wallets } from "../../application/wallets.js";
import { $, toast } from "../dom.js";

let refresh = async () => {};
let working = false;

/** Says how to refresh the whole page. */
export const onRefresh = (fn) => (refresh = fn);
export const refreshPage = () => refresh();
/** Whether an action is running, so polling doesn't redraw under it. */
export const isWorking = () => working;

export const fail = (err) => toast(err.message, { error: true });

/** Runs `action` with `button` disabled, then refreshes; a refusal is shown, not thrown. */
export async function busy(button, action) {
  button.disabled = working = true;
  try {
    await action();
    await refresh();
  } catch (err) {
    fail(err);
  } finally {
    button.disabled = working = false;
  }
}

/** Stops an action that needs a key while the wallets are locked, pointing at the unlock box. */
export function requireUnlocked() {
  if (!wallets().locked) return;
  $("unlock-passphrase")?.focus();
  throw new Error("Unlock your wallets first");
}
