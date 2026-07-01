/**
 * A rolling-window rate limiter for hub restarts.
 *
 * The supervisor restarts the hub on unexpected exit, but a hub that dies *right after* becoming
 * healthy must never respawn forever — that pegs the CPU and cooks the machine. This gate allows at
 * most `max` restarts within `windowMs`; once the budget is spent the supervisor gives up and
 * surfaces an error instead of looping. A hub that stays up longer than the window silently earns a
 * fresh budget (old attempts age out), so a one-off crash after hours of uptime still recovers.
 *
 * Deliberately pure (no Electron/Node deps) so it is unit-testable and provably bounded.
 */
export class RestartGate {
  private times: number[] = [];

  constructor(
    private readonly max: number,
    private readonly windowMs: number,
  ) {}

  /**
   * Record a restart attempt if the window has room. Returns the 1-based attempt number when
   * allowed, or `null` when the budget is exhausted for the current window.
   */
  tryAcquire(now: number = Date.now()): number | null {
    this.times = this.times.filter((t) => now - t < this.windowMs);
    if (this.times.length >= this.max) return null;
    this.times.push(now);
    return this.times.length;
  }

  /** Clear the crash history — a deliberate stop/restart earns a fresh budget. */
  reset(): void {
    this.times = [];
  }
}
