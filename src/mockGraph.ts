/** The browser mock's graph (ADR 0081): the fixture files under `src/mock/`,
 * opened by the browser core in this thread. The core answers every graph
 * question, as for a published export; an edit replaces files and the core
 * recaptures the pages they hold. With the publication worker, this is the
 * one importer of the core chunk (`browserCoreAssets` in vite.config.ts). */
import { loadBrowserCore } from "./browserCore";
import type { GraphRead } from "./publishedGraphRpc";
import type { BrowserGraph } from "./render/wasm-core/lsdoc_wasm";
import { generatedFlagFiles } from "./mockScrollFixtures";
import type { PageDto } from "./types";
import type { PageRefusal } from "./document";

const BASE_DIR = "./mock/graph/";
const FLAGS_DIR = "./mock/flags/";
const BASE = import.meta.glob<string>("./mock/graph/**/*", { query: "?raw", import: "default", eager: true });
const FLAGS = import.meta.glob<string>("./mock/flags/**/*", { query: "?raw", import: "default", eager: true });

/** The graph files (graph-relative path to text) a page URL's flags select:
 * `src/mock/graph/`, each `src/mock/flags/<flag>/` whose flag the URL names,
 * and the generated `?big` and `?manyrefs` files. */
export function mockGraphFiles(search: string): Map<string, string> {
  const files = new Map<string, string>();
  for (const [key, text] of Object.entries(BASE)) files.set(key.slice(BASE_DIR.length), text);
  for (const [key, text] of Object.entries(FLAGS)) {
    const [flag, ...path] = key.slice(FLAGS_DIR.length).split("/");
    if (new RegExp(`[?&]${flag}\\b`).test(search)) files.set(path.join("/"), text);
  }
  for (const [path, text] of Object.entries(generatedFlagFiles(search))) files.set(path, text);
  return files;
}

/** One modification time per file (Unix seconds), so recency order is fixed:
 * journals newest day first, then the other files by path, an hour apart. */
export function mockFileTimes(paths: readonly string[]): Record<string, number> {
  const base = Date.UTC(2026, 5, 15, 12, 0, 0) / 1000;
  const journal = (path: string) => path.startsWith("journals/");
  const ordered = [...paths].sort((a, b) => journal(a) !== journal(b) ? (journal(a) ? -1 : 1)
    : journal(a) ? (a < b ? 1 : a > b ? -1 : 0) : (a < b ? -1 : a > b ? 1 : 0));
  return Object.fromEntries(ordered.map((path, i) => [path, base - i * 3600]));
}

/** What `submit_page_json` answers: the key's new version, or the desktop host's refusal. */
export type MockSubmit = { written: { path: string; text: string; version: number } } | { refused: PageRefusal };

export interface MockGraph {
  read: GraphRead;
  /** The current text of a graph file. */
  text(path: string): Promise<string | undefined>;
  /** Replace (or, with null, delete) files, then recapture the pages they hold. */
  replace(changes: [string, string | null][]): Promise<void>;
  submit(key: string, dto: PageDto, version: number): Promise<MockSubmit>;
  /** Forget every edit: the next call reopens the original files, as a
   * browser reload of the preview does. */
  reset(): void;
}

/** Open the files as an editable memory graph. The core loads on the first
 * call (a session that asks no graph question never loads it), and every call
 * waits on that one open. */
export function openMockGraph(files: Map<string, string>): MockGraph {
  let texts = new Map(files);
  const encode = new TextEncoder();
  let opening: Promise<BrowserGraph> | undefined;
  const open = () => (opening ??= loadBrowserCore().then(({ BrowserGraph }) => {
    const paths = [...texts.keys()];
    return new BrowserGraph(paths, paths.map(path => encode.encode(texts.get(path)!)),
      JSON.stringify({ editable: true, times: mockFileTimes(paths) }));
  }));
  // The bindings throw the core's refusals as strings; callers get Errors, as from the publication worker.
  async function call<T>(answer: (graph: BrowserGraph) => T): Promise<T> {
    const graph = await open();
    try {
      return answer(graph);
    } catch (error) {
      throw typeof error === "string" ? new Error(error) : error;
    }
  }
  return {
    read: ([method, ...args]) => call(graph => Reflect.apply(graph[method], graph, args) as string),
    async text(path) {
      return texts.get(path);
    },
    async replace(changes) {
      await call(graph => graph.replace_files(changes.map(([path]) => path),
        changes.map(([, text]) => text === null ? null : encode.encode(text))));
      for (const [path, text] of changes) {
        if (text === null) texts.delete(path);
        else texts.set(path, text);
      }
    },
    async submit(key, dto, version) {
      const answer: MockSubmit = JSON.parse(await call(graph => graph.submit_page_json(key, JSON.stringify(dto), BigInt(version))));
      if ("written" in answer) texts.set(answer.written.path, answer.written.text);
      return answer;
    },
    reset() {
      const closing = opening;
      opening = undefined;
      texts = new Map(files);
      void closing?.then(graph => graph.free(), () => undefined);
    },
  };
}
