import assert from "node:assert/strict";
import { test } from "node:test";
import { ports } from "@application/ports";

test("asking for an adapter nobody wired is a mistake, said plainly", () => {
  assert.throws(() => ports.node, /No adapter wired for node/);
  assert.throws(
    () => ports.walletsRepository,
    /No adapter wired for walletsRepository/,
  );
});
