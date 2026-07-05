// Draws a "Session Wrapped" scorecard onto a 2D canvas — a self-contained, dependency-free renderer
// so the exact card the user sees is what downloads/shares as a crisp PNG (deterministic, high-res,
// no DOM-rasterization or web-font-embedding surprises). 1080×1920 (9:16) — Instagram-story sized.

import type { Wrapped } from "./wrapped";
import { fmtCompact, fmtDuration, fmtPercent } from "./wrapped";

export const CARD_W = 1080;
export const CARD_H = 1920;

const P = 88; // side padding
const CW = CARD_W - P * 2; // content width

const FALLBACK = 'system-ui, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif';
// Prefer Inter when the page has loaded it; harmless if the browser falls back to the system stack.
const FONT = `"Inter", ${FALLBACK}`;

const C = {
  bg: "#05060a",
  white: "#ffffff",
  frost: "#f0f0f0",
  fog: "#a1a4a5",
  steel: "#6e727a",
  blue: "#3b9eff",
  violet: "#9281f7",
  green: "#3ad389",
  hair: "rgba(255,255,255,0.09)",
  tile: "rgba(255,255,255,0.04)",
  tileBorder: "rgba(255,255,255,0.08)",
  track: "rgba(255,255,255,0.08)",
};

type Align = "left" | "right" | "center";
type Baseline = "top" | "middle" | "alphabetic";

interface TextOpts {
  size: number;
  weight?: number;
  color?: string | CanvasGradient;
  align?: Align;
  baseline?: Baseline;
  tracking?: number;
  family?: string;
}

/** Render the whole card. Sets the canvas backing size, so callers only supply the element. */
export function drawWrapped(canvas: HTMLCanvasElement, w: Wrapped): void {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  canvas.width = CARD_W;
  canvas.height = CARD_H;
  ctx.clearRect(0, 0, CARD_W, CARD_H);

  drawBackground(ctx);
  drawHeader(ctx);
  drawHero(ctx, w);
  drawVibe(ctx, w);
  drawStatGrid(ctx, w);
  drawLeaderboard(ctx, w);
  drawFooter(ctx, w);
}

function drawBackground(ctx: CanvasRenderingContext2D): void {
  ctx.fillStyle = C.bg;
  ctx.fillRect(0, 0, CARD_W, CARD_H);

  bloom(ctx, 540, -120, 960, "rgba(59,158,255,0.24)");
  bloom(ctx, 1000, 80, 820, "rgba(146,129,247,0.18)");
  bloom(ctx, 120, 2020, 760, "rgba(58,211,137,0.06)");

  // A hair-thin inner frame for a "card" edge even when posted on a white feed.
  roundRectPath(ctx, 20, 20, CARD_W - 40, CARD_H - 40, 40);
  ctx.strokeStyle = "rgba(255,255,255,0.06)";
  ctx.lineWidth = 2;
  ctx.stroke();
}

function drawHeader(ctx: CanvasRenderingContext2D): void {
  drawMark(ctx, P + 24, 116, 48);
  label(ctx, "PARLER PROTOCOL", P + 66, 116, {
    size: 26,
    weight: 600,
    color: C.blue,
    baseline: "middle",
    tracking: 3,
  });
  label(ctx, "2026", CARD_W - P, 116, {
    size: 26,
    weight: 600,
    color: C.steel,
    align: "right",
    baseline: "middle",
  });
}

function drawHero(ctx: CanvasRenderingContext2D, w: Wrapped): void {
  label(ctx, "SESSION WRAPPED", P, 208, { size: 44, weight: 700, color: C.frost, tracking: 10, baseline: "top" });

  const hero = `≈ ${fmtCompact(w.totalTokens)}`;
  ctx.font = fontStr(176, 800);
  const heroW = ctx.measureText(hero).width;
  const grad = ctx.createLinearGradient(P, 0, P + Math.max(heroW, 200), 0);
  grad.addColorStop(0, C.blue);
  grad.addColorStop(1, C.violet);
  label(ctx, hero, P, 300, { size: 176, weight: 800, baseline: "top", color: grad });

  label(ctx, "tokens exchanged", P, 512, { size: 36, weight: 500, color: C.fog, baseline: "top" });
  label(ctx, w.vibe.blurb, P, 566, { size: 28, weight: 400, color: C.steel, baseline: "top" });
}

function drawVibe(ctx: CanvasRenderingContext2D, w: Wrapped): void {
  const y = 636;
  const h = 76;
  const pad = 30;
  const emoji = w.vibe.emoji;
  const title = w.vibe.title;

  ctx.font = fontStr(36, 400);
  const emojiW = ctx.measureText(emoji).width;
  ctx.font = fontStr(32, 700);
  const titleW = ctx.measureText(title).width;
  const pillW = pad + emojiW + 16 + titleW + pad;

  roundRectPath(ctx, P, y, pillW, h, h / 2);
  ctx.fillStyle = "rgba(59,158,255,0.10)";
  ctx.fill();
  ctx.strokeStyle = "rgba(59,158,255,0.35)";
  ctx.lineWidth = 2;
  ctx.stroke();

  const cy = y + h / 2;
  label(ctx, emoji, P + pad, cy, { size: 36, weight: 400, baseline: "middle" });
  label(ctx, title, P + pad + emojiW + 16, cy, { size: 32, weight: 700, color: C.white, baseline: "middle" });
}

function drawStatGrid(ctx: CanvasRenderingContext2D, w: Wrapped): void {
  const tiles: Array<[string, string]> = [
    ["Messages", fmtCompact(w.totalMessages)],
    ["Agents", `${w.agentCount}`],
    ["Active for", fmtDuration(w.durationMs)],
    ["Tool calls", fmtCompact(w.toolCalls)],
  ];
  const gap = 28;
  const tileW = (CW - gap) / 2;
  const tileH = 172;
  const rowGap = 24;
  const y0 = 760;

  tiles.forEach(([lab, val], i) => {
    const col = i % 2;
    const row = Math.floor(i / 2);
    const x = P + col * (tileW + gap);
    const y = y0 + row * (tileH + rowGap);

    roundRectPath(ctx, x, y, tileW, tileH, 24);
    ctx.fillStyle = C.tile;
    ctx.fill();
    ctx.strokeStyle = C.tileBorder;
    ctx.lineWidth = 2;
    ctx.stroke();

    label(ctx, lab.toUpperCase(), x + 30, y + 32, { size: 24, weight: 600, color: C.steel, baseline: "top", tracking: 2 });
    label(ctx, val, x + 30, y + 74, { size: 60, weight: 700, color: C.white, baseline: "top" });
  });
}

function drawLeaderboard(ctx: CanvasRenderingContext2D, w: Wrapped): void {
  label(ctx, "WHO DID THE TALKING", P, 1176, { size: 26, weight: 600, color: C.steel, baseline: "top", tracking: 3 });

  if (!w.mvp) {
    label(ctx, "No messages in this session — yet.", P, 1300, { size: 32, weight: 500, color: C.fog, baseline: "top" });
    return;
  }

  // MVP hero row: rank chip + name + a big share percentage on the right, then a full-width bar.
  const mvp = w.mvp;
  const chipR = 30;
  const chipCx = P + chipR;
  const chipCy = 1258;
  circle(ctx, chipCx, chipCy, chipR, "rgba(59,158,255,0.16)");
  circle(ctx, chipCx, chipCy, chipR, undefined, "rgba(59,158,255,0.45)", 2);
  label(ctx, "1", chipCx, chipCy, { size: 32, weight: 800, color: C.blue, align: "center", baseline: "middle" });

  const pctText = fmtPercent(mvp.share);
  ctx.font = fontStr(64, 800);
  const pctW = ctx.measureText(pctText).width;
  label(ctx, pctText, CARD_W - P, 1236, { size: 64, weight: 800, color: C.blue, align: "right", baseline: "top" });
  label(ctx, "of the tokens", CARD_W - P, 1308, { size: 24, weight: 500, color: C.steel, align: "right", baseline: "top" });

  const nameMax = CW - (chipR * 2 + 24) - pctW - 40;
  const name = truncate(ctx, mvp.name, nameMax, fontStr(48, 700));
  label(ctx, name, chipCx + chipR + 24, 1236, { size: 48, weight: 700, color: C.white, baseline: "top" });
  label(ctx, roleLine(mvp), chipCx + chipR + 24, 1296, { size: 26, weight: 500, color: C.steel, baseline: "top" });

  bar(ctx, P, 1346, CW, mvp.share, C.blue);

  // Runner-up rows (ranks 2..5) with their own bars.
  const rest = w.topAgents.slice(1);
  let y = 1410;
  const rowH = 92;
  for (const a of rest) {
    const tokText = `≈ ${fmtCompact(a.tokens)} · ${fmtCompact(a.messages)} msg`;
    ctx.font = fontStr(24, 400);
    const tokW = ctx.measureText(tokText).width;
    const nm = truncate(ctx, `${a.rank}.  ${a.name}`, CW - tokW - 32, fontStr(30, 500));
    label(ctx, nm, P, y, { size: 30, weight: 500, color: C.frost, baseline: "top" });
    label(ctx, tokText, CARD_W - P, y, { size: 24, weight: 400, color: C.steel, align: "right", baseline: "top" });
    bar(ctx, P, y + 44, CW, a.share, "rgba(59,158,255,0.55)");
    y += rowH;
  }
}

function drawFooter(ctx: CanvasRenderingContext2D, w: Wrapped): void {
  ctx.strokeStyle = C.hair;
  ctx.lineWidth = 2;
  ctx.beginPath();
  ctx.moveTo(P, 1792);
  ctx.lineTo(CARD_W - P, 1792);
  ctx.stroke();

  label(ctx, "parlerprotocol.com", P, 1826, { size: 30, weight: 600, color: C.fog, baseline: "top" });
  label(ctx, `#${w.roomLabel}`, CARD_W - P, 1826, { size: 26, weight: 400, color: C.steel, align: "right", baseline: "top" });
}

// ---- primitives ----

function fontStr(size: number, weight = 400, family = FONT): string {
  return `${weight} ${size}px ${family}`;
}

function label(ctx: CanvasRenderingContext2D, text: string, x: number, y: number, o: TextOpts): void {
  ctx.font = fontStr(o.size, o.weight ?? 400, o.family ?? FONT);
  ctx.fillStyle = o.color ?? C.white;
  ctx.textAlign = o.align ?? "left";
  ctx.textBaseline = o.baseline ?? "alphabetic";
  // letterSpacing is widely supported in modern canvas; cast so older TS lib.dom doesn't complain.
  (ctx as unknown as { letterSpacing: string }).letterSpacing = `${o.tracking ?? 0}px`;
  ctx.fillText(text, x, y);
  (ctx as unknown as { letterSpacing: string }).letterSpacing = "0px";
}

/** Trim `text` with an ellipsis until it fits `maxW` at the given font. */
function truncate(ctx: CanvasRenderingContext2D, text: string, maxW: number, font: string): string {
  ctx.font = font;
  (ctx as unknown as { letterSpacing: string }).letterSpacing = "0px";
  if (ctx.measureText(text).width <= maxW) return text;
  let s = text;
  while (s.length > 1 && ctx.measureText(`${s}…`).width > maxW) s = s.slice(0, -1);
  return `${s}…`;
}

function roundRectPath(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number): void {
  const rr = Math.min(r, w / 2, h / 2);
  ctx.beginPath();
  ctx.moveTo(x + rr, y);
  ctx.arcTo(x + w, y, x + w, y + h, rr);
  ctx.arcTo(x + w, y + h, x, y + h, rr);
  ctx.arcTo(x, y + h, x, y, rr);
  ctx.arcTo(x, y, x + w, y, rr);
  ctx.closePath();
}

function bloom(ctx: CanvasRenderingContext2D, cx: number, cy: number, r: number, color: string): void {
  const g = ctx.createRadialGradient(cx, cy, 0, cx, cy, r);
  g.addColorStop(0, color);
  g.addColorStop(1, "rgba(0,0,0,0)");
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, CARD_W, CARD_H);
}

function circle(ctx: CanvasRenderingContext2D, cx: number, cy: number, r: number, fill?: string, stroke?: string, lineWidth = 1): void {
  ctx.beginPath();
  ctx.arc(cx, cy, r, 0, Math.PI * 2);
  if (fill) {
    ctx.fillStyle = fill;
    ctx.fill();
  }
  if (stroke) {
    ctx.strokeStyle = stroke;
    ctx.lineWidth = lineWidth;
    ctx.stroke();
  }
}

/** A rounded progress bar: full-width track + a fill sized to `share` (0..1, min 4% so it never vanishes). */
function bar(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, share: number, fill: string): void {
  const h = 14;
  roundRectPath(ctx, x, y, w, h, h / 2);
  ctx.fillStyle = C.track;
  ctx.fill();
  const fw = Math.max(w * 0.04, Math.min(1, Math.max(0, share)) * w);
  roundRectPath(ctx, x, y, fw, h, h / 2);
  ctx.fillStyle = fill;
  ctx.fill();
}

/** The Parler orbit mark: an electric-blue ring with a satellite dot around a violet nucleus. */
function drawMark(ctx: CanvasRenderingContext2D, cx: number, cy: number, size: number): void {
  const r = size / 2;
  ctx.save();
  ctx.lineWidth = Math.max(3, size * 0.09);
  ctx.strokeStyle = C.blue;
  circle(ctx, cx, cy, r, undefined, C.blue, ctx.lineWidth);
  circle(ctx, cx, cy, size * 0.15, C.violet);
  const a = -Math.PI / 4;
  circle(ctx, cx + r * Math.cos(a), cy + r * Math.sin(a), size * 0.11, C.blue);
  ctx.restore();
}

function roleLine(a: { role?: string; messages: number }): string {
  return a.role ? `${a.role} · ${fmtCompact(a.messages)} msg` : `${fmtCompact(a.messages)} messages`;
}
