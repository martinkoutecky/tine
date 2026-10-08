// Docs-build half of the static-export sheets (family 7): compute the Guide's sheet
// blocks with the SAME TS evaluator the app calls (`src/sheet/staticExport.ts`), so the
// website Guide and a user's published site show sheets identically and no second
// implementation exists (I-12). Run by scripts/build-guide-demo.mjs through vite-node:
//   vite-node scripts/sheet-export-cli.ts <inputs.json> <exports.json>
// The clock is pinned to SOURCE_DATE_EPOCH when set, so the checked-in demo is reproducible.
import fs from "node:fs";
import { initParser } from "../src/render/parse";
import { computeSheetExports, type SheetInput } from "../src/sheet/staticExport";

const [inputsFile, exportsFile] = process.argv.slice(2);
if (!inputsFile || !exportsFile) throw new Error("usage: sheet-export-cli.ts <inputs.json> <exports.json>");
await initParser();
const inputs = JSON.parse(fs.readFileSync(inputsFile, "utf8")) as SheetInput[];
const epoch = process.env.SOURCE_DATE_EPOCH;
const now = epoch ? new Date(Number(epoch) * 1000) : new Date();
// The Guide is built for the default task workflow.
const exports = computeSheetExports(inputs, "now", now);
fs.writeFileSync(exportsFile, JSON.stringify(exports));
console.log(`computed ${exports.length} sheets of ${inputs.length} candidate blocks`);
