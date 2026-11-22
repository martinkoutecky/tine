import { afterEach, expect, it, onTestFinished, vi } from "vitest";
import { readBounded, validateSnapshot } from "./publishedBackend";
import { testPublishedBackend } from "./publishedBackend.methods.test.support";
import { parsePublishedPermalinkHash, publishedPermalinkHash } from "./publishedPermalink";

const files = {
  "pages/Public.md": "alias:: Alias\n\n- TODO one\n",
  "logseq/config.edn": '{:default-home {:page "Public"}}',
};
const uuid = "11111111-1111-4111-8111-111111111111";

afterEach(() => vi.unstubAllGlobals());

it("opens one captured graph and runs unrecorded queries over the shared pages", async () => {
  const snapshot = { schema: 2, name: "Example", exported_at: "", files: Object.entries(files).map(([path, text]) => ({ path, text })) };
  const fetch = vi.fn(async () => Response.json(snapshot));
  vi.stubGlobal("fetch", fetch);
  vi.resetModules();
  const { loadPublishedGraph, publishedBackend } = await import("./publishedBackend");
  const opened = await loadPublishedGraph("snapshot.json");
  onTestFinished(() => opened.graph.free());
  expect(await loadPublishedGraph("snapshot.json")).toBe(opened);
  expect(fetch).toHaveBeenCalledTimes(1);
  const api = publishedBackend();
  expect(api.graphBindingGeneration()).toBe(1);
  expect(await api.loadGraph("")).toMatchObject({ kind: "loaded", meta: { root: "Example", default_home: "Public" } });
  expect((await api.pageInventory()).entries.map((entry) => entry.name)).toEqual(["Alias", "Public", "TODO"]);
  expect(await api.getPage("Alias", "page")).toMatchObject({ id: "pages/Public.md", read_only: true });
  for (const [task, total] of [["TODO", 1], ["DONE", 0]] as const) {
    const parsed = await api.parseQuery(`(task ${task})`, "macro_query");
    expect(await api.queryRun(parsed.query, parsed.view)).toMatchObject({ total });
  }
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
