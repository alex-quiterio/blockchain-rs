// Running an action from a button: one at a time, with the page refreshed after and any refusal
// shown. Panels use it; main.ts says how the page refreshes.

import type { Wallets } from "@domain/wallets/wallets";
import { showError } from "@ui/dom";

export class Actions {
  #refresh: () => Promise<void>;
  #wallets: Wallets;
  #working = false;

  /** `refresh` redraws the whole page and shows its own failures. */
  constructor(refresh: () => Promise<void>, wallets: Wallets) {
    this.#refresh = refresh;
    this.#wallets = wallets;
  }

  /** Whether an action is running, so polling doesn't redraw under it. */
  get working(): boolean {
    return this.#working;
  }

  refreshPage(): Promise<void> {
    return this.#refresh();
  }

  /** Runs `action` with `button` disabled, then refreshes; a refusal is shown, not thrown. */
  async busy(
    button: HTMLButtonElement,
    action: () => Promise<void>,
  ): Promise<void> {
    button.disabled = this.#working = true;
    try {
      await action();
      await this.#refresh();
    } catch (err) {
      showError(err);
    } finally {
      button.disabled = this.#working = false;
    }
  }

  /** Stops an action that needs a key while the wallets are locked, pointing at the unlock box. */
  requireUnlocked(): void {
    if (!this.#wallets.locked) return;
    document.getElementById("unlock-passphrase")?.focus();
    throw new Error("Unlock your wallets first");
  }
}
