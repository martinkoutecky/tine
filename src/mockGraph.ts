/** The browser mock's graph (ADR 0081): the fixture files under `src/mock/`,
 * opened by the browser core in this thread. The core answers every graph
 * question, as for a published export; an edit replaces files and the core
 * recaptures the pages they hold. With the publication worker, this is the
 * one importer of the core chunk (`browserCoreAssets` in vite.config.ts). */
import { loadBrowserCore } from "./browserCore";
import type { GraphRead } from "./publishedGraphRpc";
import type { BrowserGraph } from "./render/wasm-core/lsdoc_wasm";
import { generatedFlagFiles } from "./mockScrollFixtures";
import type { PageDto, PageRead, ResolvedPage } from "./types";
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

/** A call on a session a window reload retired: refused before the core runs. */
export class RetiredGraph extends Error {}

export type MockGraph = ReturnType<typeof openMockGraph>;
export type MockGraphSession = ReturnType<typeof openSession>;

/** Open the files as an editable memory graph, one session per window load:
 * `read` (the app's reads) and `current()` (which a request captures when it
 * starts) answer the current session, and `reset()` retires it, as a browser
 * reload of the preview does, so the next session reopens the original files. */
export function openMockGraph(files: Map<string, string>) {
  let current = openSession(files, 1);
  return {
    read: (call => current.read(call)) as GraphRead,
    current: () => current,
    reset() { current.retire(); current = openSession(files, current.session + 1); },
  };
}

/** One session's graph, the one owner of which graph a request writes. Its
 * core loads on its first call (a session that asks no graph question never
 * loads it). Each call runs in one turn of the core, so no `await` separates a
 * check from its write, and once the session is retired each call rejects
 * with `RetiredGraph` before the core runs. */
function openSession(files: Map<string, string>, session: number) {
  const encode = (text: string) => new TextEncoder().encode(text);
  let opening: Promise<BrowserGraph> | undefined;
  let retired = false;
  // The bindings throw the core's refusals as strings; callers get Errors, as from the publication worker.
  async function use<T>(answer: (graph: BrowserGraph) => T): Promise<T> {
    if (retired) throw new RetiredGraph();
    const graph = await (opening ??= loadBrowserCore().then(({ BrowserGraph }) => {
      const paths = [...files.keys()];
      return new BrowserGraph(paths, paths.map(path => encode(files.get(path)!)),
        JSON.stringify({ editable: true, times: mockFileTimes(paths) }));
    }));
    if (retired) throw new RetiredGraph();
    try {
      return answer(graph);
    } catch (error) {
      throw typeof error === "string" ? new Error(error) : error;
    }
  }
  return {
    session,
    get retired() { return retired; },
    read: (([method, ...args]) => use(graph => Reflect.apply(graph[method], graph, args) as string)) as GraphRead,
    /** The page at `path` (null: none) and the core's version of the file, read together. */
    page: (path: string) => use(graph => ({ page: JSON.parse(graph.get_page_by_path_json(path)).page as PageRead | null,
      version: Number(graph.version(path)) })),
    resolve: (name: string, kind: "journal" | "page") =>
      use(graph => JSON.parse(graph.resolve_page_json(name, kind === "journal")) as ResolvedPage),
    /** Submit at the file's current version: its new version, or the desktop host's refusal. */
    submit: (key: string, dto: PageDto) => use((graph): { version: number } | { refused: PageRefusal } => {
      const answer = JSON.parse(graph.submit_page_json(key, JSON.stringify(dto), graph.version(key)));
      return "written" in answer ? { version: answer.written.version } : answer;
    }),
    /** Whether the core deleted the file at `version` (a stale version deletes nothing). */
    deleteFile: (path: string, version: number) =>
      use(graph => "deleted" in JSON.parse(graph.delete_file_json(path, BigInt(version)))),
    /** Whether the core admitted the delete by name (the desktop's one target). */
    deleteName: (name: string, kind: "journal" | "page", expectedPath?: string) =>
      use(graph => "deleted" in JSON.parse(graph.delete_page_json(name, kind === "journal", expectedPath))),
    /** Replace (null: delete) one file, then recapture the pages it holds. */
    write: (path: string, text: string | null) =>
      use(graph => void graph.replace_files([path], [text === null ? null : encode(text)])),
    retire() {
      retired = true;
      void opening?.then(graph => graph.free(), () => undefined);
    },
  };
}
