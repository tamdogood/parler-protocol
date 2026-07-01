// Generates the app + tray icons as PNGs from raw pixels — no binary assets to hand-manage.
// Produces build/icon.png (1024, electron-builder → .icns) and build/trayTemplate{,@2x}.png.
import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const buildDir = join(root, "build");
const resDir = join(root, "resources");
mkdirSync(buildDir, { recursive: true });
mkdirSync(resDir, { recursive: true });

// ---- minimal PNG encoder (RGBA, 8-bit) ----
const CRC = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();
function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) c = CRC[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const typeBuf = Buffer.from(type, "ascii");
  const body = Buffer.concat([typeBuf, data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body), 0);
  return Buffer.concat([len, body, crc]);
}
function encodePng(width, height, rgba) {
  const sig = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // color type RGBA
  // 10,11,12 = 0 (deflate/adaptive/no-interlace)
  const stride = width * 4;
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (stride + 1)] = 0; // filter: none
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, y * stride + stride);
  }
  const idat = deflateSync(raw, { level: 9 });
  return Buffer.concat([sig, chunk("IHDR", ihdr), chunk("IDAT", idat), chunk("IEND", Buffer.alloc(0))]);
}

// ---- drawing helpers ----
function canvas(size) {
  return { size, data: Buffer.alloc(size * size * 4) };
}
function set(cv, x, y, [r, g, b, a]) {
  if (x < 0 || y < 0 || x >= cv.size || y >= cv.size) return;
  const i = (y * cv.size + x) * 4;
  // simple src-over onto existing
  const da = cv.data[i + 3] / 255;
  const sa = a / 255;
  const outA = sa + da * (1 - sa);
  const blend = (s, d) => (outA === 0 ? 0 : Math.round((s * sa + d * da * (1 - sa)) / outA));
  cv.data[i] = blend(r, cv.data[i]);
  cv.data[i + 1] = blend(g, cv.data[i + 1]);
  cv.data[i + 2] = blend(b, cv.data[i + 2]);
  cv.data[i + 3] = Math.round(outA * 255);
}
function hex(h, a = 255) {
  return [parseInt(h.slice(1, 3), 16), parseInt(h.slice(3, 5), 16), parseInt(h.slice(5, 7), 16), a];
}
// Anti-aliased coverage of a value crossing a threshold over ~1px.
function aa(d, edge) {
  return Math.max(0, Math.min(1, 0.5 - (d - edge)));
}
function withAlpha(color, cov) {
  return [color[0], color[1], color[2], Math.round((color[3] ?? 255) * cov)];
}

// Rounded-square black app tile with an electric-blue orbit glyph + violet core.
function drawAppIcon(size) {
  const cv = canvas(size);
  const pad = size * 0.085;
  const r = size * 0.225; // corner radius
  const min = pad;
  const max = size - pad;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      // rounded-rect signed distance
      const cx = Math.min(Math.max(x, min + r), max - r);
      const cy = Math.min(Math.max(y, min + r), max - r);
      const inCorner = x < min + r || x > max - r ? 1 : 0;
      const inCornerY = y < min + r || y > max - r ? 1 : 0;
      let inside;
      if (inCorner && inCornerY) {
        const d = Math.hypot(x - cx, y - cy);
        inside = aa(d, r);
      } else {
        inside = x >= min && x <= max && y >= min && y <= max ? 1 : 0;
      }
      if (inside > 0) set(cv, x, y, withAlpha(hex("#060607"), inside));
    }
  }
  // subtle top surface-lift
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const i = (y * cv.size + x) * 4;
      if (cv.data[i + 3] === 0) continue;
      const t = 1 - y / size;
      const lift = Math.round(18 * Math.max(0, t - 0.4));
      cv.data[i] = Math.min(255, cv.data[i] + lift);
      cv.data[i + 1] = Math.min(255, cv.data[i + 1] + lift);
      cv.data[i + 2] = Math.min(255, cv.data[i + 2] + lift);
    }
  }
  const c = size / 2;
  const blue = hex("#3b9eff");
  const violet = hex("#9281f7");
  // orbit ring
  const ringR = size * 0.26;
  const ringW = size * 0.028;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const d = Math.abs(Math.hypot(x - c, y - c) - ringR);
      const cov = aa(d, ringW);
      if (cov > 0) set(cv, x, y, withAlpha(blue, cov * 0.9));
    }
  }
  // core dot
  const coreR = size * 0.072;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const d = Math.hypot(x - c, y - c);
      const cov = aa(d, coreR);
      if (cov > 0) set(cv, x, y, withAlpha(violet, cov));
    }
  }
  // orbiting satellite dot (upper-right on the ring)
  const ang = -Math.PI / 4;
  const sx = c + Math.cos(ang) * ringR;
  const sy = c + Math.sin(ang) * ringR;
  const satR = size * 0.045;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const d = Math.hypot(x - sx, y - sy);
      const cov = aa(d, satR);
      if (cov > 0) set(cv, x, y, withAlpha(blue, cov));
    }
  }
  return cv;
}

// macOS template tray icon: black-with-alpha orbit glyph (the OS tints it for light/dark menu bars).
function drawTray(size) {
  const cv = canvas(size);
  const c = size / 2;
  const black = [0, 0, 0, 255];
  const ringR = size * 0.32;
  const ringW = Math.max(1, size * 0.06);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const d = Math.abs(Math.hypot(x - c, y - c) - ringR);
      const cov = aa(d, ringW);
      if (cov > 0) set(cv, x, y, withAlpha(black, cov));
    }
  }
  const coreR = size * 0.14;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const cov = aa(Math.hypot(x - c, y - c), coreR);
      if (cov > 0) set(cv, x, y, withAlpha(black, cov));
    }
  }
  return cv;
}

function save(dir, name, cv) {
  const buf = encodePng(cv.size, cv.size, cv.data);
  writeFileSync(join(dir, name), buf);
  console.log(`wrote ${dir === buildDir ? "build" : "resources"}/${name} (${cv.size}x${cv.size}, ${buf.length} bytes)`);
}

// App icon → build/ (electron-builder generates the .icns from it, build-time only).
save(buildDir, "icon.png", drawAppIcon(1024));
// Tray template icons → resources/ (shipped as extraResources; loaded at runtime).
save(resDir, "trayTemplate.png", drawTray(16));
save(resDir, "trayTemplate@2x.png", drawTray(32));
