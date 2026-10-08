import assert from "node:assert/strict";
import fs from "node:fs";

const PNG_SIGNATURE = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
const COLOR_TYPES_WITHOUT_ALPHA = new Set([0, 2, 3]);

export function assertOpaquePng(path, label) {
  const bytes = fs.readFileSync(path);
  assert.ok(bytes.subarray(0, 8).equals(PNG_SIGNATURE), `${label} is not a PNG`);
  assert.equal(bytes.subarray(12, 16).toString("ascii"), "IHDR", `${label} lacks IHDR`);

  const colorType = bytes[25];
  assert.ok(
    COLOR_TYPES_WITHOUT_ALPHA.has(colorType),
    `${label} has an alpha-bearing PNG color type (${colorType})`,
  );

  for (let offset = 8; offset + 12 <= bytes.length; ) {
    const length = bytes.readUInt32BE(offset);
    const type = bytes.subarray(offset + 4, offset + 8).toString("ascii");
    assert.notEqual(type, "tRNS", `${label} contains a PNG transparency chunk`);
    offset += 12 + length;
  }
}
