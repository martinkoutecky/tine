/// <reference lib="webworker" />
import { loadBrowserCore } from "./browserCore";
import type { GraphReply, GraphRequest } from "./publishedGraphRpc";
import { validateSnapshot } from "./publishedSnapshot";
import { decodeShareDocument, QuickShareError } from "./quickShare/quickShare";
import type { BrowserGraph } from "./render/wasm-core/lsdoc_wasm";

const worker = self as unknown as DedicatedWorkerGlobalScope;

let graph: BrowserGraph;
let methods: Set<string>;

async function answer({ id, method, args }: GraphRequest): Promise<GraphReply> {
  if (method === "ping") return { id, json: "null" };
  if (method === "open") {
    const [buffer, isShare] = args;
    const bytes = new Uint8Array(buffer);
    const { name, files } = isShare
      ? decodeShareDocument(bytes)
      : validateSnapshot(JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes)));
    try {
      const { BrowserGraph } = await loadBrowserCore();
      methods = new Set(Object.getOwnPropertyNames(BrowserGraph.prototype).filter(name => name.endsWith("_json")));
      const encode = new TextEncoder();
      graph = new BrowserGraph(files.map(file => file.path), files.map(file => encode.encode(file.text)));
      return { id, json: JSON.stringify(name) };
    } catch (error) {
      return { id, error: String(error), fatal: true };
    }
  }
  if (!methods.has(method)) return { id, error: `Unknown graph read: ${method}` };
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
