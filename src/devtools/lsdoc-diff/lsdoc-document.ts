// The lsdoc side of the comparison: parse a WHOLE FILE (raw graph file text) to
// the observable projection {blocks, refs} — the same thing graph-check gets from
// the `lsdoc-parse` CLI. lsdoc is stateless and O(n), so it runs on the main
// thread (no worker, no timeout needed).
//
// Backed by the wasm `parse_document_json` export (lsdoc-wasm, lsdoc ≥ v0.4.2), a
// thin wrapper over `lsdoc::parse_format`. The wasm bytes are already loaded at app
// boot by parse.ts's `initParser()`; we only gate on it being ready.
import { lsdoc_tag, parse_document_json } from "../../render/wasm/lsdoc_wasm.js";
import { parserReady } from "../../render/parse";
import type { Projection } from "./mldoc-client";

/** True once the wasm parser is initialized (it is, after app boot). Gates the
 *  panel's divergence scan + the lsdoc bench column. */
export function lsdocDocumentAvailable(): boolean {
  return parserReady();
}

/** Exact released lsdoc tag embedded in the parser used for this comparison. */
export function lsdocVersion(): string {
  return parserReady() ? lsdoc_tag() : "unavailable";
}

/** Parse a whole file with lsdoc into the {blocks, refs} projection. Caller must
 *  gate on `lsdocDocumentAvailable()`; a parse error throws and is handled per-file
 *  by the orchestrator. */
export function parseLsdocDocument(text: string, isOrg: boolean): Projection {
  return JSON.parse(parse_document_json(text, isOrg)) as Projection;
}
