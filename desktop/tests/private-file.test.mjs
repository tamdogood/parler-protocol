import assert from "node:assert/strict";
import { chmodSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { writePrivateFile } from "../src/main/private-file.ts";

test("private files replace a loose inode atomically with owner-only permissions", () => {
  const directory = mkdtempSync(join(tmpdir(), "parler-private-file-"));
  const path = join(directory, "sessions.json");
  try {
    writeFileSync(path, "old", { mode: 0o644 });
    chmodSync(path, 0o644);
    writePrivateFile(path, "new capability");

    assert.equal(readFileSync(path, "utf8"), "new capability");
    if (process.platform !== "win32") {
      assert.equal(statSync(path).mode & 0o777, 0o600);
    }
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
