import assert from "node:assert/strict";
import process from "node:process";
import { createCanvas, loadImage } from "@napi-rs/canvas";
import { assertOpaquePng } from "./lib/opaque-png.mjs";

const [, , expectedPath, bundledPath] = process.argv;
// Xcode may normalize alpha and color encoding when compiling an asset catalog.
// Compare visible RGB artwork with tolerance; Tauri's template icon is separated
// from Tine's by a mean error above 100, so this remains a strong identity gate.
const MAX_MEAN_ABSOLUTE_RGB_ERROR = 8;
if (!expectedPath || !bundledPath) {
  throw new Error(
    "usage: verify-ios-app-icon.mjs <tracked-icon.png> <decoded-bundled-icon.png>",
  );
}

assertOpaquePng(expectedPath, "tracked iOS icon");
assertOpaquePng(bundledPath, "signed IPA primary icon");

async function rgba(path) {
  const image = await loadImage(path);
  const canvas = createCanvas(image.width, image.height);
  const context = canvas.getContext("2d");
  context.drawImage(image, 0, 0);
  return {
    width: image.width,
    height: image.height,
    pixels: Buffer.from(
      context.getImageData(0, 0, image.width, image.height).data,
    ),
  };
}

const [expected, bundled] = await Promise.all([
  rgba(expectedPath),
  rgba(bundledPath),
]);
assert.deepEqual(
  { width: bundled.width, height: bundled.height },
  { width: expected.width, height: expected.height },
  "the signed IPA primary icon dimensions differ from Tine's tracked icon",
);
let absoluteRgbError = 0;
let maximumRgbError = 0;
for (let offset = 0; offset < expected.pixels.length; offset += 4) {
  for (let channel = 0; channel < 3; channel += 1) {
    const error = Math.abs(
      bundled.pixels[offset + channel] - expected.pixels[offset + channel],
    );
    absoluteRgbError += error;
    maximumRgbError = Math.max(maximumRgbError, error);
  }
}
const meanAbsoluteRgbError =
  absoluteRgbError / (expected.width * expected.height * 3);
assert.ok(
  meanAbsoluteRgbError <= MAX_MEAN_ABSOLUTE_RGB_ERROR,
  `the signed IPA primary icon is not visually Tine's tracked icon: mean absolute RGB error ${meanAbsoluteRgbError.toFixed(3)} exceeds ${MAX_MEAN_ABSOLUTE_RGB_ERROR}`,
);
console.log(
  `verified ${bundled.width}x${bundled.height} signed IPA icon artwork (mean absolute RGB error ${meanAbsoluteRgbError.toFixed(3)}, maximum channel error ${maximumRgbError})`,
);
