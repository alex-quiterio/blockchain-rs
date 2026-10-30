// Running an action from a button: one at a time, with the page refreshed after and any refusal
// shown. Panels use it; main.ts says how the page refreshes.

import { wallets } from "@application/wallets";
import { toast } from "@ui/dom";

let refresh: () => Promise<void> = async () => {};
let working = false;

/** Says how to refresh the whole page. */
export const onRefresh = (fn: () => Promise<void>): void => {
  refresh = fn;
};
export const refreshPage = (): Promise<void> => refresh();
/** Whether an action is running, so polling doesn't redraw under it. */
export const isWorking = (): boolean => working;

export const fail = (err: unknown): void =>
  toast(err instanceof Error ? err.message : String(err), { error: true });

/** Runs `action` with `button` disabled, then refreshes; a refusal is shown, not thrown. */
export async function busy(
  button: HTMLButtonElement,
  action: () => Promise<void>,
): Promise<void> {
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
export function requireUnlocked(): void {
  if (!wallets().locked) return;
  document.getElementById("unlock-passphrase")?.focus();
  throw new Error("Unlock your wallets first");
}
