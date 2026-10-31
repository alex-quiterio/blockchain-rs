import assert from "node:assert/strict";
import { test } from "node:test";
import {
  COINBASE,
  STAKE,
  UNSTAKE,
  feesOf,
  kindOf,
  signingPayload,
} from "@domain/transfers";

const base = { from: "aa", to: "bb", amount: 5, nonce: 2 };

test("a transfer's kind follows from where its coins come from and go", () => {
  assert.equal(kindOf({ ...base, from: COINBASE }), "coinbase");
  assert.equal(kindOf({ ...base, to: STAKE }), "stake");
  assert.equal(kindOf({ ...base, to: UNSTAKE }), "unstake");
  assert.equal(kindOf(base), "transfer");
});

test("fees add up, counting none as zero", () => {
  assert.equal(feesOf([base, { ...base, fee: 3 }, { ...base, fee: 4 }]), 7);
  assert.equal(feesOf([]), 0);
});

test("what's signed names a fee, commitment or memo only when there is one", () => {
  assert.equal(signingPayload(base), "aa|bb|5|2");
  assert.equal(
    signingPayload({ ...base, fee: 0 }),
    "aa|bb|5|2",
    "a zero fee is left out",
  );
  assert.equal(signingPayload({ ...base, fee: 3 }), "aa|bb|5|2|fee=3");
  assert.equal(
    signingPayload({ ...base, to: STAKE, commitment: "cc" }),
    "aa|stake|5|2|commit=cc",
  );
  assert.equal(
    signingPayload({ ...base, fee: 1, memo: "dd" }),
    "aa|bb|5|2|fee=1|memo=dd",
  );
});
