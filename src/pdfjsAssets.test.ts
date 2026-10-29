import { expect, it } from "vitest";
import fs from "node:fs/promises";
import path from "node:path";
import { copyPdfjsAssets } from "../scripts/vite-pdfjs-assets";
import { openjpegBuildDir } from "../scripts/build-pdfjs-openjpeg.mjs";

it("ships the pinned PDF decoders, their fallbacks, fonts and licences offline", async () => {
  await fs.mkdir("artifacts", { recursive: true });
  const target = await fs.mkdtemp(path.resolve("artifacts/pdfjs-assets-test-"));
  try {
    await copyPdfjsAssets(target);
    for (const file of ["wasm/openjpeg_nowasm_fallback.js",
      "wasm/jbig2.wasm", "wasm/jbig2_nowasm_fallback.js", "wasm/qcms_bg.wasm",
      "wasm/LICENSE_JBIG2", "wasm/LICENSE_QCMS",
      "standard_fonts/FoxitSerif.pfb", "standard_fonts/LICENSE_FOXIT",
      "cmaps/Adobe-Japan1-UCS2.bcmap", "iccs/CGATS001Compat-v2-micro.icc", "iccs/LICENSE"]) {
      const expected = await fs.readFile(path.join("node_modules/pdfjs-dist", file));
      expect(await fs.readFile(path.join(target, "pdfjs", file)), file).toEqual(expected);
    }
    for (const file of ["openjpeg.wasm", "LICENSE_OPENJPEG", "BUILD_OPENJPEG.json"]) {
      expect(await fs.readFile(path.join(target, "pdfjs/wasm", file))).toEqual(await fs.readFile(path.join(openjpegBuildDir, file)));
    }
    const receipt = JSON.parse(await fs.readFile(path.join(target, "pdfjs/wasm/BUILD_OPENJPEG.json"), "utf8"));
    expect(receipt.relaxedSimd).toBe(false);
    expect(receipt.initialMemoryBytes).toBe(16 * 1024 * 1024);
    expect(receipt.maximumMemoryBytes).toBe(2 * 1024 * 1024 * 1024);
  } finally { await fs.rm(target, { recursive: true, force: true }); }
}, 120_000);
