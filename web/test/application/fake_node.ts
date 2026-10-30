// A node in memory for the use cases to talk to: it answers from `answers`, records every call,
// and refuses whatever it wasn't told to answer.

import type { Account, NodePort } from "@application/ports";
import type { SavedWallets, WalletsRepository } from "@domain/wallets/wallets";

export interface Call {
  method: keyof NodePort;
  args: unknown[];
}

/** What the fake answers a method with: the method's own arguments and result, but not a promise. */
export type Answers = Partial<{
  [K in keyof NodePort]: (
    ...args: Parameters<NodePort[K]>
  ) => Awaited<ReturnType<NodePort[K]>>;
}>;

export function fakeNode(answers: Answers) {
  const calls: Call[] = [];
  const node = new Proxy({} as NodePort, {
    get(_, method: keyof NodePort) {
      return (...args: unknown[]) => {
        calls.push({ method, args });
        const answer = answers[method] as
          ((...a: unknown[]) => unknown) | undefined;
        if (!answer)
          return Promise.reject(
            new Error(`the fake node wasn't asked to answer ${method}`),
          );
        // A refusal comes back as a rejected promise, as it does from the real node.
        return new Promise((resolve) => resolve(answer(...args)));
      };
    },
  });
  return {
    node,
    calls,
    called: (method: keyof NodePort) =>
      calls.filter((c) => c.method === method),
  };
}

export const account = (fields: Partial<Account> = {}): Account => ({
  address: "aa".repeat(32),
  balance: 100,
  staked: 0,
  next_nonce: 0,
  next_epoch: 0,
  reveals_left: null,
  commitment_due: null,
  ...fields,
});

/** A wallets repository that keeps nothing between runs. */
export const emptyRepository = (): WalletsRepository => {
  let saved: SavedWallets = { accounts: [], vault: null, legacy: [] };
  return {
    load: () => structuredClone(saved),
    save: ({ accounts, vault }) => {
      saved = { ...saved, accounts, vault };
    },
    forgetLegacy: () => {},
  };
};
