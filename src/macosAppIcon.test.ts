import { readFileSync } from "node:fs";
import { inflateSync } from "node:zlib";
import { expect, it } from "vitest";

// GH #747: the macOS icon needs Apple's icon-grid margin, or the Dock shows Tine
// larger than every other app. Reads the 1024 px PNG (type ic10) inside icon.icns.
function ic10Png(icns: Buffer): Buffer {
  for (let at = 8; at < icns.length; at += icns.readUInt32BE(at + 4)) {
    if (icns.toString("latin1", at, at + 4) === "ic10") return icns.subarray(at + 8, at + icns.readUInt32BE(at + 4));
  }
  throw new Error("icon.icns has no 1024 px (ic10) image");
}

/** Alpha of 8-bit RGBA, non-interlaced PNG pixels (enough for this one file). */
function alphaAt(png: Buffer, points: [number, number][]): number[] {
  const width = png.readUInt32BE(16);
  expect([png[24], png[25], png[28]]).toEqual([8, 6, 0]);
  const chunks: Buffer[] = [];
  for (let at = 8; at < png.length; at += png.readUInt32BE(at) + 12) {
    if (png.toString("latin1", at + 4, at + 8) === "IDAT") chunks.push(png.subarray(at + 8, at + 8 + png.readUInt32BE(at)));
  }
  const raw = inflateSync(Buffer.concat(chunks)), stride = width * 4, rows: Buffer[] = [];
  for (let y = 0; y * (stride + 1) < raw.length; y++) {
    const filter = raw[y * (stride + 1)], line = Buffer.from(raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1)));
    const prev = rows[y - 1] ?? Buffer.alloc(stride);
    for (let i = 0; i < stride; i++) {
      const a = i >= 4 ? line[i - 4] : 0, b = prev[i], c = i >= 4 ? prev[i - 4] : 0;
      const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
      const predictor = [0, a, b, (a + b) >> 1, pa <= pb && pa <= pc ? a : pb <= pc ? b : c][filter];
      line[i] = (line[i] + predictor) & 0xff;
    }
    rows.push(line);
  }
  return points.map(([x, y]) => rows[y][x * 4 + 3]);
}

it("GH #747: the macOS icon leaves Apple's icon-grid margin around the artwork", () => {
  const png = ic10Png(readFileSync("src-tauri/icons/icon.icns"));
  expect(png.readUInt32BE(16)).toBe(1024);
  // Edge midpoints are transparent margin; the centre is the opaque artwork.
  expect(alphaAt(png, [[40, 512], [984, 512], [512, 40], [512, 984], [512, 512]])).toEqual([0, 0, 0, 0, 255]);
});
