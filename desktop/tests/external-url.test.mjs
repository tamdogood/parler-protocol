import assert from "node:assert/strict";
import test from "node:test";
import { safeExternalUrl } from "../src/main/external-url.ts";

test("external navigation accepts HTTPS and rejects executable or local schemes", () => {
  assert.equal(safeExternalUrl("https://example.com/docs"), "https://example.com/docs");
  assert.equal(safeExternalUrl("http://example.com"), null);
  assert.equal(safeExternalUrl("file:///tmp/payload"), null);
  assert.equal(safeExternalUrl("javascript:alert(1)"), null);
  assert.equal(safeExternalUrl("not a url"), null);
});
