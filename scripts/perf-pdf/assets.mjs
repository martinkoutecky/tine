// PDF perf harness inputs: the three-document corpus and the stock pdf.js
// 4.10.38 release (the reference viewer). Neither is committed; each is
// downloaded once into a cache directory and verified against a pinned SHA-256.
//
//   node scripts/perf-pdf/assets.mjs          # fetch/verify, print the paths
//
// Cache: $TINE_PDF_PERF_CACHE, else $XDG_CACHE_HOME/tine-pdf-perf, else
// ~/.cache/tine-pdf-perf. PDF_CORPUS / PDFJS_DIST point the harness at
// existing copies instead (then nothing is fetched or verified here).
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { zipExtractAll } from "../lib/e2e-zip.mjs";

export const CORPUS = [
  { id: "S1", file: "arikan-0807.3917v5.pdf", url: "https://arxiv.org/pdf/0807.3917v5",
    sha256: "36046a14f967b7b8be88a9e7beb4dd1de475a1d689b97dc43a05a3279e1c2f4d" },
  { id: "S2", file: "bv_cvxbook.pdf", url: "https://web.stanford.edu/~boyd/cvxbook/bv_cvxbook.pdf",
    sha256: "40d976c83c18cce1900eff8c41bd5ad408c102b813af39d05ff85678ccf8d76e" },
  { id: "S3", file: "sourcebookinmath1929smit.pdf",
    url: "https://archive.org/download/sourcebookinmath1929smit/sourcebookinmath1929smit.pdf",
    sha256: "d5d748c37f625422891c5b02903cdbf5d7cfde32c42013db19af187c602bb180" },
];
export const PDFJS = {
  version: "4.10.38", file: "pdfjs-4.10.38-dist.zip",
  url: "https://github.com/mozilla/pdf.js/releases/download/v4.10.38/pdfjs-4.10.38-dist.zip",
  sha256: "32bdd9c5198b77dbaa8f02de81f34476888b3abdd64b9fb5f607f81f01487e6a",
};

export function cacheDir() {
  if (process.env.TINE_PDF_PERF_CACHE) return path.resolve(process.env.TINE_PDF_PERF_CACHE);
  return path.join(process.env.XDG_CACHE_HOME || path.join(os.homedir(), ".cache"), "tine-pdf-perf");
}

const sha256 = (file) => crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");

/** `dest` with the pinned hash, downloading it when missing; a mismatch is fatal. */
async function fetchVerified(url, dest, want) {
  if (fs.existsSync(dest)) {
    const got = sha256(dest);
    if (got === want) return dest;
    throw new Error(`${dest}: SHA-256 ${got}, expected ${want}. Delete it to re-download.`);
  }
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  console.error(`fetching ${url}`);
  const res = await fetch(url, { redirect: "follow", headers: { "User-Agent": "tine-pdf-perf-harness" } });
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  const bytes = Buffer.from(await res.arrayBuffer());
  const got = crypto.createHash("sha256").update(bytes).digest("hex");
  if (got !== want) throw new Error(`${url}: SHA-256 ${got}, expected ${want}; not saved`);
  const part = `${dest}.part-${process.pid}`;
  fs.writeFileSync(part, bytes);
  fs.renameSync(part, dest);
  return dest;
}

/** { corpus, dist }: directories holding the verified corpus and the extracted pdf.js dist. */
export async function ensureAssets() {
  if (process.env.PDF_CORPUS && process.env.PDFJS_DIST) {
    return { corpus: path.resolve(process.env.PDF_CORPUS), dist: path.resolve(process.env.PDFJS_DIST) };
  }
  const root = cacheDir();
  const corpus = process.env.PDF_CORPUS ? path.resolve(process.env.PDF_CORPUS) : path.join(root, "corpus");
  if (!process.env.PDF_CORPUS) {
    for (const doc of CORPUS) await fetchVerified(doc.url, path.join(corpus, doc.file), doc.sha256);
  }
  let dist = process.env.PDFJS_DIST ? path.resolve(process.env.PDFJS_DIST) : null;
  if (!dist) {
    const zip = await fetchVerified(PDFJS.url, path.join(root, PDFJS.file), PDFJS.sha256);
    dist = path.join(root, `pdfjs-dist-${PDFJS.version}`);
    const marker = path.join(dist, `.extracted-${PDFJS.sha256}`);
    if (!fs.existsSync(marker)) {
      fs.rmSync(dist, { recursive: true, force: true });
      zipExtractAll(zip, dist);
      fs.writeFileSync(marker, "");
    }
  }
  return { corpus, dist };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { corpus, dist } = await ensureAssets();
  console.log(`PDF_CORPUS=${corpus}\nPDFJS_DIST=${dist}`);
}
