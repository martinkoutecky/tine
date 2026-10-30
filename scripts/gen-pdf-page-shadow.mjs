// Generates the 9-slice page-shadow image that src/styles/pdf-workspace.css
// uses for `.pdf-page::before` (PDF9-5), as a base64 PNG data payload.
//
//   node scripts/gen-pdf-page-shadow.mjs        # prints the base64 payload
//
// It reproduces `box-shadow: 0 2px 10px rgba(0, 0, 0, 0.25)` (the page shadow
// before PDF9-5) analytically: a Gaussian-blurred rectangle is separable, so each
// pixel's alpha is alpha * Fx * Fy with Fx = Phi((x - x0) / sigma) - Phi((x - x1) / sigma).
// The image is drawn at 2x (so it stays crisp at DPR 2): 24 px slices around a
// 16 px middle, CSS `border-image-slice: 24` + `border-image-width: 12px`.
import zlib from "node:zlib";
import { pathToFileURL } from "node:url";

export const SHADOW = { scale: 2, slice: 24, middle: 16, blur: 10, dy: 2, alpha: 0.25 };

function erf(x) {
  // Abramowitz-Stegun 7.1.26, |error| < 1.5e-7
  const s = Math.sign(x), a = Math.abs(x), t = 1 / (1 + 0.3275911 * a);
  const y = 1 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * Math.exp(-a * a);
  return s * y;
}
const phi = (z) => 0.5 * (1 + erf(z / Math.SQRT2));

/** Alpha (0..1) of the shadow at image-pixel index i along one axis (slice < i < size - slice is the middle). */
function axisAlpha(i, shift, { slice, middle, blur, scale }) {
  const sigma = (blur * scale) / 2;
  const far = 1e4; // far edge: effectively infinite
  let p; // position relative to the page edge nearest this slice (page covers p >= 0 .. far)
  if (i < slice) p = i - slice + 0.5;
  else if (i < slice + middle) p = far / 2;
  else p = far + (i - slice - middle) + 0.5;
  return phi((p - shift) / sigma) - phi((p - far - shift) / sigma);
}

export function shadowPng(opts = SHADOW) {
  const { slice, middle, alpha, dy, scale } = opts;
  const size = 2 * slice + middle;
  const raw = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0; // PNG filter: none
    const ay = axisAlpha(y, dy * scale, opts);
    for (let x = 0; x < size; x++) {
      const a = Math.round(255 * alpha * axisAlpha(x, 0, opts) * ay);
      raw[y * (size * 4 + 1) + 1 + x * 4 + 3] = a; // RGB stays 0 (black)
    }
  }
  const crcTable = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
  const crc = (buf) => { let c = 0xffffffff; for (const b of buf) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
  const chunk = (type, data) => {
    const body = Buffer.concat([Buffer.from(type), data]);
    const out = Buffer.alloc(body.length + 8);
    out.writeUInt32BE(data.length, 0); body.copy(out, 4); out.writeUInt32BE(crc(body), body.length + 4);
    return out;
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0); ihdr.writeUInt32BE(size, 4); ihdr[8] = 8; ihdr[9] = 6; // 8-bit RGBA
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", ihdr),
    chunk("IDAT", zlib.deflateSync(raw, { level: 9 })), chunk("IEND", Buffer.alloc(0))]);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  console.log(shadowPng().toString("base64"));
}
