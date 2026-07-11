// Comparison worker — runs MLDOC (Logseq's reference parser) off the main thread
// for the "Help improve Tine" diff panel. Why a worker, and why mldoc-only:
//
//  • mldoc can be pathologically slow / quadratic on adversarial input (that's the
//    bench showcase). On the UI thread a slow parse would freeze Tine; here the
//    orchestrator can `worker.terminate()` on a per-file timeout — the browser
//    analog of graph-check.mjs's per-file SIGKILL subprocess guard.
//  • mldoc leaks global state across parses in ONE realm (lsdoc CLAUDE.md: `$$$`
//    before `$$$$` flips the second's result). graph-check spawns a FRESH
//    subprocess per authoritative parse to avoid this. Our analog: one worker =
//    one mldoc realm; the orchestrator gets "fresh" state by using a fresh worker
//    (terminate + respawn) for re-verify / minimize / anonymize probes.
//  • lsdoc is NOT here: it is stateless and O(n) by construction (it can't hang and
//    needs no isolation), so the orchestrator parses it on the main thread via the
//    existing wasm — no second wasm instance, no cross-thread hop.
//
// mldoc is loaded lazily via dynamic import of the vendored bundle URL (passed in
// the init message) so the 413 KB costs nothing until the panel actually runs.
// It attaches `Mldoc` to the worker global on load. normalize/refs are the SAME
// pure libs the differential gate uses, bundled into this worker chunk.
import { normalizeAst } from "./vendor/normalize.mjs";
import { extractRefs } from "./vendor/refs.mjs";

type Format = "md" | "org";

interface MldocApi {
  parseJson(input: string, cfg: string): string;
}

let mldoc: MldocApi | null = null;
let loadError: string | null = null;

// mldoc config = OG's graph-parser default, per harness/oracle.mjs. Must stay in
// sync with the gate so an in-app diff and the CI gate agree on divergences.
function cfg(format: Format): string {
  return JSON.stringify({
    toc: false,
    parse_outline_only: false,
    heading_number: false,
    keep_line_break: true,
    format: format === "org" ? "Org" : "Markdown",
    heading_to_list: false,
    export_md_remove_options: [],
  });
}

async function ensureMldoc(url: string): Promise<void> {
  if (mldoc || loadError) return;
  try {
    // Native dynamic import runs the js_of_ocaml IIFE (side-effect: sets
    // globalThis.Mldoc). @vite-ignore keeps Vite from bundling the asset — it must
    // load lazily as its own request. Proven to run in a browser realm (jsdom +
    // Logseq's Chromium renderer); real-WebKitGTK check is the task-5 smoke test.
    await import(/* @vite-ignore */ url);
    const api = (self as unknown as { Mldoc?: MldocApi }).Mldoc;
    if (!api || typeof api.parseJson !== "function") {
      loadError = "mldoc loaded but exposed no parseJson";
      return;
    }
    mldoc = api;
  } catch (e) {
    loadError = `mldoc failed to load: ${String(e).split("\n")[0]}`;
  }
}

export interface ParseRequest {
  type: "parse";
  id: string;
  mldocUrl: string;
  text: string;
  format: Format;
}

export interface ParseResponse {
  id: string;
  ok: boolean;
  projection?: { blocks: unknown[]; refs: { page: string[]; block: string[] } };
  parseMicros?: number;
  status?: "load-error" | "parse-error";
  detail?: string;
}

self.onmessage = async (ev: MessageEvent<ParseRequest>) => {
  const msg = ev.data;
  if (!msg || msg.type !== "parse") return;
  await ensureMldoc(msg.mldocUrl);
  if (!mldoc) {
    reply({ id: msg.id, ok: false, status: "load-error", detail: loadError || "mldoc unavailable" });
    return;
  }
  const start = performance.now();
  try {
    const ast = JSON.parse(mldoc.parseJson(msg.text, cfg(msg.format)));
    const projection = {
      blocks: normalizeAst(ast) as unknown[],
      refs: extractRefs(ast, msg.format),
    };
    const parseMicros = Math.round((performance.now() - start) * 1000);
    reply({ id: msg.id, ok: true, projection, parseMicros });
  } catch (e) {
    reply({ id: msg.id, ok: false, status: "parse-error", detail: String(e).split("\n")[0] });
  }
};

function reply(r: ParseResponse) {
  (self as unknown as { postMessage(m: ParseResponse): void }).postMessage(r);
}
