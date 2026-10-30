import assert from "node:assert/strict";
import { test } from "node:test";
import { fromHex, sha256Hex, toHex } from "@domain/encoding";

test("hex goes there and back", () => {
  const bytes = new Uint8Array([0, 1, 15, 16, 255]);
  assert.equal(toHex(bytes), "00010f10ff");
  assert.deepEqual(fromHex("00010f10ff"), bytes);
});

test("SHA-256 of text matches the standard test vector", async () => {
  assert.equal(
    await sha256Hex("abc"),
    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
  );
});
