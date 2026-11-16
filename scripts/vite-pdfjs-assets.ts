import fs from "node:fs";
import fsp from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import type { Plugin } from "vite";
import { buildOpenjpeg } from "./build-pdfjs-openjpeg.mjs";

const source = fileURLToPath(new URL("../node_modules/pdfjs-dist/", import.meta.url));
export const pdfjsAssetDirectories = ["cmaps", "standard_fonts", "wasm", "iccs"] as const;

/** Keep the decoder filenames and licences together. pdf.js resolves these
 * lazily, including its JS decoder fallbacks, relative to the supplied URLs. */
export async function copyPdfjsAssets(outDir: string) {
  for (const directory of pdfjsAssetDirectories) {
    await fsp.cp(path.join(source, directory), path.join(outDir, "pdfjs", directory), { recursive: true });
  }
  const decoder = buildOpenjpeg();
  for (const file of ["openjpeg.wasm", "LICENSE_OPENJPEG", "BUILD_OPENJPEG.json"]) {
    await fsp.copyFile(path.join(decoder, file), path.join(outDir, "pdfjs/wasm", file));
  }
}

export function pdfjsAssets(): Plugin {
  let outDir: string;
  return {
    name: "pdfjs-offline-assets",
    configResolved(config) { outDir = path.resolve(config.root, config.build.outDir); },
    configureServer(server) {
      const decoder = buildOpenjpeg();
      server.middlewares.use((req, res, next) => {
        const match = /^\/pdfjs\/(cmaps|standard_fonts|wasm|iccs)\/([^/?]+)(?:\?.*)?$/.exec(req.url ?? "");
        if (!match || match[2] === "." || match[2] === "..") return next();
        const custom = match[1] === "wasm" && ["openjpeg.wasm", "LICENSE_OPENJPEG", "BUILD_OPENJPEG.json"].includes(match[2]);
        const file = custom ? path.join(decoder, match[2]) : path.join(source, match[1], match[2]);
        if (!fs.existsSync(file) || !fs.statSync(file).isFile()) return next();
        res.setHeader("Content-Type", file.endsWith(".wasm") ? "application/wasm"
          : file.endsWith(".js") ? "text/javascript" : "application/octet-stream");
        fs.createReadStream(file).pipe(res);
      });
    },
    async writeBundle() { await copyPdfjsAssets(outDir); },
  };
}
