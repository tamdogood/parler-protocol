import { closeSync, fsyncSync, openSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { randomUUID } from "node:crypto";

/** Atomically replace a local capability/config file with an owner-only inode. */
export function writePrivateFile(path: string, contents: string): void {
  const temporary = `${path}.tmp.${process.pid}.${randomUUID()}`;
  let fd: number | null = null;
  try {
    fd = openSync(temporary, "wx", 0o600);
    writeFileSync(fd, contents, "utf8");
    fsyncSync(fd);
    closeSync(fd);
    fd = null;
    renameSync(temporary, path);
  } catch (error) {
    if (fd !== null) {
      try {
        closeSync(fd);
      } catch {
        // Preserve the original write error.
      }
    }
    rmSync(temporary, { force: true });
    throw error;
  }
}
