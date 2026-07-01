import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { createServer } from "node:net";
import { mkdirSync, readFileSync, statSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { EventEmitter } from "node:events";
import type { HubStatus, HubStorage } from "../shared/types";
import { hubBinary, hubDbPath, hubBlobDir, joinSecretPath, dataDir } from "./paths";
import { loadSettings } from "./settings";

const HEALTH_INTERVAL_MS = 3000;
const START_TIMEOUT_MS = 15000;
const LOG_RING = 500;
const MAX_RESTARTS = 5;

/** Find a free TCP port on 127.0.0.1, starting at `start` and stepping up. */
function findFreePort(start: number, tries = 20): Promise<number> {
  return new Promise((resolve, reject) => {
    const attempt = (port: number, left: number) => {
      const srv = createServer();
      srv.once("error", () => {
        srv.close();
        if (left <= 0) reject(new Error(`no free port near ${start}`));
        else attempt(port + 1, left - 1);
      });
      srv.once("listening", () => {
        srv.close(() => resolve(port));
      });
      srv.listen(port, "127.0.0.1");
    };
    attempt(start, tries);
  });
}

async function probeHealth(url: string): Promise<boolean> {
  const ctrl = new AbortController();
  const t = setTimeout(() => ctrl.abort(), 2000);
  try {
    const res = await fetch(`${url}/health`, { signal: ctrl.signal });
    return res.ok;
  } catch {
    return false;
  } finally {
    clearTimeout(t);
  }
}

/**
 * Supervises the bundled `parler-hub` child process: spawn with the user's settings, poll `/health`
 * to reach `running`, keep a rolling log buffer, and restart on unexpected exit (with a cap). The
 * durable SQLite + blob store live in the app's userData, so a restart never loses data.
 */
export class HubSupervisor extends EventEmitter {
  private child: ChildProcessWithoutNullStreams | null = null;
  private logs: string[] = [];
  private healthTimer: NodeJS.Timeout | null = null;
  private restarts = 0;
  private stopping = false;
  private status: HubStatus = {
    phase: "stopped",
    url: null,
    pid: null,
    mode: "private",
    name: "",
    startedAt: null,
    error: null,
    healthy: false,
  };

  getStatus(): HubStatus {
    return { ...this.status };
  }

  getLogs(): string[] {
    return [...this.logs];
  }

  private setStatus(patch: Partial<HubStatus>): void {
    this.status = { ...this.status, ...patch };
    this.emit("status", this.getStatus());
  }

  private log(line: string): void {
    for (const raw of line.split(/\r?\n/)) {
      const trimmed = raw.replace(/\s+$/, "");
      if (!trimmed) continue;
      this.logs.push(trimmed);
      if (this.logs.length > LOG_RING) this.logs.shift();
      this.emit("log", trimmed);
    }
  }

  /** Start the hub if not already running. Idempotent. */
  async start(): Promise<HubStatus> {
    if (this.child) return this.getStatus();
    this.stopping = false;
    const settings = loadSettings();
    mkdirSync(dataDir(), { recursive: true });
    mkdirSync(hubBlobDir(), { recursive: true });

    let port: number;
    try {
      port = await findFreePort(settings.hubPort);
    } catch (e) {
      this.setStatus({ phase: "error", error: (e as Error).message });
      return this.getStatus();
    }
    const url = `http://127.0.0.1:${port}`;
    const mode = settings.hubPublic ? "public" : "private";

    const args = [
      "--addr",
      `127.0.0.1:${port}`,
      "--db",
      hubDbPath(),
      "--name",
      settings.hubName,
      "--blob-dir",
      hubBlobDir(),
    ];
    if (settings.hubPublic) {
      args.push("--public");
    } else {
      // Private hub: auto-generate + persist a join secret so a LAN-reachable hub stays closed.
      args.push("--join-secret-file", joinSecretPath());
    }

    this.setStatus({
      phase: "starting",
      url,
      pid: null,
      mode,
      name: settings.hubName,
      startedAt: Date.now(),
      error: null,
      healthy: false,
    });
    this.log(`$ parler-hub ${args.join(" ")}`);

    let child: ChildProcessWithoutNullStreams;
    try {
      child = spawn(hubBinary(), args, {
        env: { ...process.env, RUST_LOG: process.env.RUST_LOG || "info" },
      });
    } catch (e) {
      this.setStatus({ phase: "error", error: `failed to launch hub: ${(e as Error).message}` });
      return this.getStatus();
    }
    this.child = child;
    this.setStatus({ pid: child.pid ?? null });

    child.stdout.on("data", (b: Buffer) => this.log(b.toString()));
    child.stderr.on("data", (b: Buffer) => this.log(b.toString()));
    child.on("exit", (code, signal) => this.onExit(code, signal));
    child.on("error", (e) => {
      this.log(`hub process error: ${e.message}`);
      this.setStatus({ phase: "error", error: e.message });
    });

    // Poll for health until the port answers, then flip to running.
    const deadline = Date.now() + START_TIMEOUT_MS;
    const waitHealthy = async (): Promise<void> => {
      while (Date.now() < deadline && this.child === child && !this.stopping) {
        if (await probeHealth(url)) {
          this.restarts = 0;
          this.setStatus({ phase: "running", healthy: true });
          this.beginHealthLoop(url, child);
          return;
        }
        await new Promise((r) => setTimeout(r, 400));
      }
      if (this.child === child && this.status.phase !== "running" && !this.stopping) {
        this.setStatus({ phase: "error", error: "hub did not become healthy in time" });
      }
    };
    void waitHealthy();
    return this.getStatus();
  }

  private beginHealthLoop(url: string, child: ChildProcessWithoutNullStreams): void {
    if (this.healthTimer) clearInterval(this.healthTimer);
    this.healthTimer = setInterval(async () => {
      if (this.child !== child) return;
      const ok = await probeHealth(url);
      if (ok !== this.status.healthy) this.setStatus({ healthy: ok });
    }, HEALTH_INTERVAL_MS);
  }

  private onExit(code: number | null, signal: NodeJS.Signals | null): void {
    this.log(`hub exited (code=${code ?? "null"}, signal=${signal ?? "null"})`);
    this.child = null;
    if (this.healthTimer) {
      clearInterval(this.healthTimer);
      this.healthTimer = null;
    }
    if (this.stopping) {
      this.setStatus({ phase: "stopped", pid: null, healthy: false, startedAt: null });
      return;
    }
    // Unexpected exit: restart with a cap so a persistently-broken binary doesn't loop forever.
    if (this.restarts < MAX_RESTARTS) {
      this.restarts += 1;
      this.log(`restarting hub (attempt ${this.restarts}/${MAX_RESTARTS})…`);
      this.setStatus({ phase: "starting", pid: null, healthy: false });
      setTimeout(() => void this.start(), 800 * this.restarts);
    } else {
      this.setStatus({
        phase: "error",
        pid: null,
        healthy: false,
        error: "hub crashed repeatedly — check the logs and port availability",
      });
    }
  }

  /** Stop the hub (graceful SIGTERM, then SIGKILL). */
  async stop(): Promise<HubStatus> {
    this.stopping = true;
    if (this.healthTimer) {
      clearInterval(this.healthTimer);
      this.healthTimer = null;
    }
    const child = this.child;
    if (!child) {
      this.setStatus({ phase: "stopped", pid: null, healthy: false, startedAt: null });
      return this.getStatus();
    }
    await new Promise<void>((resolve) => {
      const killTimer = setTimeout(() => {
        try {
          child.kill("SIGKILL");
        } catch {
          /* already gone */
        }
      }, 4000);
      child.once("exit", () => {
        clearTimeout(killTimer);
        resolve();
      });
      try {
        child.kill("SIGTERM");
      } catch {
        clearTimeout(killTimer);
        resolve();
      }
    });
    this.child = null;
    this.setStatus({ phase: "stopped", pid: null, healthy: false, startedAt: null });
    return this.getStatus();
  }

  async restart(): Promise<HubStatus> {
    await this.stop();
    this.stopping = false;
    return this.start();
  }

  /** Synchronous shutdown for app quit (best-effort). */
  shutdownSync(): void {
    this.stopping = true;
    if (this.child) {
      try {
        this.child.kill("SIGTERM");
      } catch {
        /* ignore */
      }
    }
  }

  /** The join secret for a private local hub, or null (public hub / not yet generated). */
  joinSecret(): string | null {
    if (this.status.mode === "public") return null;
    try {
      const s = readFileSync(joinSecretPath(), "utf8").trim();
      return s || null;
    } catch {
      return null;
    }
  }

  /** On-disk footprint of the hub's durable data. */
  storage(): HubStorage {
    const db = hubDbPath();
    let dbBytes = 0;
    for (const suffix of ["", "-wal", "-shm"]) {
      try {
        dbBytes += statSync(db + suffix).size;
      } catch {
        /* missing is fine */
      }
    }
    let blobBytes = 0;
    try {
      const dir = hubBlobDir();
      for (const name of readdirSync(dir)) {
        try {
          blobBytes += statSync(join(dir, name)).size;
        } catch {
          /* ignore */
        }
      }
    } catch {
      /* no blobs yet */
    }
    return { dbPath: db, dbBytes, blobBytes, dataDir: dataDir() };
  }
}
