/// <reference lib="webworker" />
import { loadBrowserCore } from "./browserCore";
import type { GraphReply, GraphRequest } from "./publishedGraphRpc";
import { validateSnapshot } from "./publishedSnapshot";
import { decodeShareDocument, QuickShareError } from "./quickShare/quickShare";
import type { BrowserGraph } from "./render/wasm-core/lsdoc_wasm";
import limits from "../docs/contracts/quick-share-limits.json";

const worker = self as unknown as DedicatedWorkerGlobalScope;

let graph: BrowserGraph;
let methods: Set<string>;
let shareQueryLocations: Set<string> | undefined;

async function answer(request: GraphRequest): Promise<GraphReply> {
  const { id, method, args } = request;
  if (method === "ping") return { id, json: "null" };
  if (method === "open") {
    const [buffer, isShare] = args;
    const bytes = new Uint8Array(buffer);
    const { name, exported_at, files } = isShare
      ? decodeShareDocument(bytes)
      : validateSnapshot(JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes)));
    try {
      const { BrowserGraph } = await loadBrowserCore();
      // Reads only: the published graph is read-only, so its one `_json` write is no read.
      methods = new Set(Object.getOwnPropertyNames(BrowserGraph.prototype)
        .filter(name => name.endsWith("_json") && name !== "submit_page_json"));
      const encode = new TextEncoder();
      const paths = files.map(file => file.path);
      const sources = files.map(file => encode.encode(file.text));
      graph = new BrowserGraph(paths, sources, isShare ? JSON.stringify({ share: { name, exported_at } }) : undefined);
      shareQueryLocations = isShare ? new Set() : undefined;
      return { id, json: JSON.stringify(name) };
    } catch (error) {
      if (isShare && typeof error === "string" && error.startsWith("share-too-large:")) {
        return { id, error, shareReason: "too-large" };
      }
      return { id, error: String(error), fatal: true };
    }
  }
  if (!methods.has(method)) return { id, error: `Unknown graph read: ${method}` };
  // Every live renderer parses before running, even when its run shares an
  // in-flight answer. Admit source locations here and retain only admitted keys.
  const location = request.queryLocation;
  if (method === "parse_query_json" && shareQueryLocations && location !== undefined && !shareQueryLocations.has(location)) {
    if (shareQueryLocations.size === limits.viewer.queries) {
      return { id, error: "Live query location limit reached", queryLimit: true };
    }
    shareQueryLocations.add(location);
  }
  try {
    return { id, json: Reflect.apply(graph[method], graph, args) };
  } catch (error) {
    // The bindings throw Rust refusals as strings. A JS exception can invalidate the WASM handle.
    return typeof error === "string" ? { id, error } : { id, error: String(error), fatal: true };
  }
}

async function respond(data: GraphRequest) {
  let reply: GraphReply;
  try {
    reply = await answer(data);
  } catch (error) {
    reply = error instanceof QuickShareError
      ? { id: data.id, error: error.message, shareReason: error.reason }
      : { id: data.id, error: String(error) };
  }
  worker.postMessage(reply);
}

function holdLifetimeLock(name: string) {
  return new Promise<void>((acquired, reject) => {
    void worker.navigator.locks.request(name, () => {
      acquired();
      return new Promise<never>(() => {});
    }).catch(reject);
  });
}

worker.onmessage = async ({ data }: MessageEvent<GraphRequest>) => {
  try {
    if (data.method === "open" && data.args[2]) await holdLifetimeLock(data.args[2]);
    await respond(data);
  } catch (error) {
    worker.postMessage({ id: data.id, error: String(error), fatal: true } satisfies GraphReply);
  }
};
worker.onmessageerror = () => { throw new Error("Could not receive a browser graph request"); };
