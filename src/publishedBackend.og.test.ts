import { afterEach, expect, it, vi } from "vitest";
import { readBounded, validateSnapshot } from "./publishedSnapshot";
import { testPublishedBackend } from "./publishedBackend.methods.test.support";
import { parsePublishedPermalinkHash, publishedPermalinkHash } from "./publishedPermalink";
import type { GraphReply, GraphRequest } from "./publishedGraphRpc";

const files = {
  "pages/Public.md": "alias:: Alias\n\n- TODO one\n",
  "logseq/config.edn": '{:default-home {:page "Public"}}',
};
const uuid = "11111111-1111-4111-8111-111111111111";

afterEach(() => vi.unstubAllGlobals());

it.each([false, true])("reports silent worker loss while opening (Web Locks: %s)", async (webLocks) => {
  const { openPublishedGraphWorker } = await import("./publishedGraphWorker");
  const postMessage = vi.fn();
  const terminate = vi.fn();
  vi.stubGlobal("Worker", class { postMessage = postMessage; terminate = terminate; });
  vi.stubGlobal("navigator", { locks: webLocks ? { request: vi.fn() } : undefined });
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "performance"] });
  try {
    const opened = openPublishedGraphWorker(new Uint8Array(), false);
    void opened.catch(() => {});
    await vi.advanceTimersByTimeAsync(180_000);
    expect(postMessage.mock.calls.map(([request]) => request.method)).toEqual(["open", "ping"]);
    await expect(opened).rejects.toThrow(/Worker stopped responding/);
    expect(terminate).toHaveBeenCalledTimes(1);
  } finally {
    vi.useRealTimers();
  }
});

it("transfers one snapshot, pairs overlapping reads with their answers and drains them on worker failure", async () => {
  const received: GraphRequest[] = [];
  const detached: number[] = [];
  class TestWorker {
    static instance: TestWorker;
    onmessage?: (event: { data: GraphReply }) => void;
    onerror?: (event: { message: string; preventDefault(): void }) => void;
    terminate = vi.fn();
    constructor() { TestWorker.instance = this; }
    postMessage(request: GraphRequest, transfer: Transferable[]) {
      received.push(structuredClone(request, { transfer }));
      if (request.method === "open") {
        detached.push(request.args[0].byteLength);
        this.reply({ id: request.id, json: JSON.stringify("Example") });
      }
    }
    reply(data: GraphReply) { this.onmessage?.({ data }); }
    crash() { this.onerror?.({ message: "Worker failed", preventDefault() {} }); }
  }
  vi.stubGlobal("Worker", TestWorker);
  const snapshot = { schema: 2, name: "Example", exported_at: "", files: [] };
  const fetch = vi.fn(async () => Response.json(snapshot));
  vi.stubGlobal("fetch", fetch);
  vi.resetModules();
  const { loadPublishedGraph } = await import("./publishedBackend");
  const [opened, same] = await Promise.all([loadPublishedGraph(), loadPublishedGraph()]);
  const worker = TestWorker.instance;
  try {
    expect(opened).toBe(same);
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(detached).toEqual([0]);
    expect(received[0]).toMatchObject({ method: "open" });
    const first = opened.read(["page_inventory_json"]);
    const second = opened.read(["journal_feed_page_json", 1, 2n, false]);
    expect(received.slice(1).map(({ method }) => method)).toEqual(["page_inventory_json", "journal_feed_page_json"]);
    worker.reply({ id: received[2].id, json: "2" });
    worker.reply({ id: received[1].id, json: "1" });
    expect(await Promise.all([first, second])).toEqual(["1", "2"]);

    const pending = opened.read(["get_page_json", "Example", false]);
    worker.crash();
    const failure = await opened.failure;
    await expect(pending).rejects.toBe(failure);
    await expect(opened.read(["get_page_json", "Example", false])).rejects.toBe(failure);
    expect(worker.terminate).toHaveBeenCalledTimes(1);
    expect(received).toHaveLength(4);
  } finally {
    worker.crash();
  }
});

it("runs unrecorded queries over the shared pages", async () => {
  const api = testPublishedBackend(files, "Example");
  expect(api.graphBindingGeneration()).toBe(1);
  expect(await api.loadGraph("")).toMatchObject({ kind: "loaded", meta: { root: "Example", default_home: "Public" } });
  expect((await api.pageInventory()).entries.map((entry) => entry.name)).toEqual(["Alias", "Public", "TODO"]);
  expect(await api.getPage("Alias", "page")).toMatchObject({ id: "pages/Public.md", read_only: true });
  for (const [task, total] of [["TODO", 1], ["DONE", 0]] as const) {
    const parsed = await api.parseQuery(`(task ${task})`, "macro_query");
    expect(await api.queryRun(parsed.query, parsed.view)).toMatchObject({ total });
  }
});

it("uses ordinary current-page context and candidates on an export home", async () => {
  const api = testPublishedBackend({
    "pages/Public.md": "- referred to [[Focus]]\n",
    "pages/Export.md": "- [[Export]]\n",
    "logseq/config.edn": '{:default-home {:page "Export"}}',
  }, "Example");
  const { query, view } = await api.parseQuery(
    '{:query [:find (pull ?b [*]) :in $ ?p :where [?page :block/name ?p] [?b :block/refs ?page]] :inputs [:current-page]}',
    "macro_query",
  );
  const ordinary = await api.queryRun(query, view, { current_page: "Export" });
  expect(ordinary).toMatchObject({ total: 1, groups: [{ page: "Export", blocks: [{ raw: "[[Export]]" }] }] });
  const pages = await api.parseQuery("@page and name != 'Unused'", "macro_tql");
  expect(await api.queryRun(pages.query, pages.view)).toMatchObject({ pages: expect.arrayContaining([{ path: "pages/Export.md", name: "Export", kind: "page", properties: [] }]) });
});

it("refuses writes and keeps the public fragment grammar", async () => {
  const api = testPublishedBackend(files);
  const dto = { name: "Public", kind: "page" as const, title: "Public", pre_block: null, blocks: [] };
  await expect(api.pageSubmit(1, 1, "pages/Public.md", dto, 1, null, ["replace-page"])).rejects.toThrow("read-only published export");
  await expect(api.pageMove(1, 2, ["pages/Public.md", dto, 1], ["pages/Other.md", dto, 1], ["move-blocks"])).rejects.toThrow("read-only published export");
  await expect(api.pageDelete(1, "Public", "page")).rejects.toThrow("read-only published export");
  for (const target of [{ kind: "page", page: "Notes ü" }, { kind: "block", block: uuid }] as const) {
    expect(parsePublishedPermalinkHash(publishedPermalinkHash(target))).toEqual(target);
  }
});

it("requires source text in a generation-2 document", () => {
  const snapshot = { schema: 2, name: "Example", exported_at: "" };
  expect(() => validateSnapshot({ ...snapshot, schema: 1, pages: [] })).toThrow(/schema/);
  expect(() => validateSnapshot({ ...snapshot, files: [{ path: "pages/X.md", text: [] }] })).toThrow(/Invalid/);
});

it("admits up to 256 distinct live query locations per opened share", async () => {
  await (await import("./browserCore")).loadBrowserCore();
  const worker = { onmessage: undefined as ((event: { data: GraphRequest }) => Promise<void>) | undefined,
    postMessage: vi.fn<(reply: GraphReply) => void>(), crypto: globalThis.crypto, performance: globalThis.performance };
  vi.stubGlobal("self", worker);
  await import("./publishedGraph.worker");
  let id = 0;
  const request = async (message: Omit<GraphRequest, "id"> & { queryLocation?: string }) => {
    await worker.onmessage!({ data: { ...message, id: ++id } as GraphRequest });
    return worker.postMessage.mock.lastCall![0];
  };
  const open = () => request({ method: "open", args: [new TextEncoder().encode(JSON.stringify({
    quick_share: 2, snapshot: { schema: 2, name: "Example", exported_at: "",
      files: Object.entries(files).map(([path, text]) => ({ path, text })) },
  })).buffer, true, undefined] });
  const parse = (queryLocation: string) => request({
    method: "parse_query_json", args: ["(task TODO)", "macro_query"], queryLocation,
  });
  expect(await open()).toMatchObject({ json: '"Example"' });
  for (let i = 0; i < 256; i++) expect(await parse(`source/${i}`)).toHaveProperty("json");
  expect(await parse("opening/begin")).toHaveProperty("queryLimit", true);
  expect(await parse("macro/0/inner/0")).toHaveProperty("queryLimit", true);
  expect(await parse("source/0")).toHaveProperty("json");
  expect(await request({ method: "page_inventory_json", args: [] })).toHaveProperty("json");
  expect(await open()).toHaveProperty("json", '"Example"');
  expect(await parse("opening/begin")).toHaveProperty("json");
});

it("bounds the preview and returns detached metadata for an authored ID", async () => {
  const text = `- Root #tag\n  id:: ${uuid}\n  key:: value\n` + "  - child\n".repeat(20000);
  const api = testPublishedBackend({ "pages/Preview.md": text });
  const preview = await api.previewBlock(uuid, 1);
  expect(preview?.truncated).toBe(20000);
  const root = preview!.group.blocks[0];
  expect(root.children).toEqual([]);
  root.tags![0] = "changed";
  const next = await api.previewBlock(uuid, 1);
  expect(next?.group.blocks[0].tags).toEqual(["tag"]);
});

it("lists real block referrers with shallow rows and breadcrumbs", async () => {
  const api = testPublishedBackend({ "pages/References.md": [
    `- Source\n  id:: ${uuid}`,
    `- Parent ((${uuid}))\n  - Child ((${uuid}))`,
    `- Inline code \`((${uuid}))\` only`,
    "- \`\`\`\n  " + `((${uuid}))` + "\n  \`\`\`",
  ].join("\n") });
  const groups = await api.getBlockReferrers(uuid);
  const blocks = groups.flatMap((group) => group.blocks);
  expect(blocks.map(block => block.raw)).toEqual([`Parent ((${uuid}))`, `Child ((${uuid}))`]);
  expect(blocks.every(block => !block.children.length)).toBe(true);
  expect(blocks[1].breadcrumb).toEqual([`Parent ((${uuid}))`]);
});

const bytesOf = (n: number) => new Uint8Array(n).fill(7);
const TEST_ASSET_MAX_BYTES = 10;
const TOO_SMALL_MAX_BYTES = TEST_ASSET_MAX_BYTES - 1;
const respond = (body: Uint8Array, init: ResponseInit = {}) => new Response(body as unknown as BodyInit, init);

it("rejects a missing or refused asset instead of answering empty bytes", async () => {
  vi.stubGlobal("fetch", vi.fn(async () => new Response("nope", { status: 404 })));
  const api = testPublishedBackend(files);
  await expect(api.readAsset("gone.png", TEST_ASSET_MAX_BYTES)).rejects.toThrow(/gone\.png.*404/);
  await expect(api.readAsset("../escape.png", TEST_ASSET_MAX_BYTES)).rejects.toThrow(/outside|not a file/);
  await expect(api.streamAsset("/etc/passwd")).rejects.toThrow(/not a file/);
});

it("honours the caller's byte cap on an asset, by declared length and by streamed length", async () => {
  const api = testPublishedBackend(files);
  vi.stubGlobal("fetch", vi.fn(async () => respond(bytesOf(10), { headers: { "content-length": "10" } })));
  expect((await api.readAsset("ok.bin", TEST_ASSET_MAX_BYTES)).byteLength).toBe(10);
  await expect(api.readAsset("big.bin", TOO_SMALL_MAX_BYTES)).rejects.toThrow(/larger than 9/);
  // No declared length: the running total refuses and cancels the stream.
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (let i = 0; i < 5; i++) controller.enqueue(bytesOf(4));
      controller.close();
    },
  });
  await expect(readBounded(new Response(stream), 9, "asset s")).rejects.toThrow(/larger than 9/);
});

it("refuses an oversized snapshot document before parsing it", async () => {
  vi.resetModules();
  const { loadPublishedGraph } = await import("./publishedBackend");
  vi.stubGlobal("fetch", vi.fn(async () => respond(new TextEncoder().encode("{}"), { headers: { "content-length": String(1 << 30) } })));
  await expect(loadPublishedGraph("snapshot.json")).rejects.toThrow(/larger than/);
});
