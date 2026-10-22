import { afterEach, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { REFRESH_READ_COMMANDS } from "./refreshReads";

// UI-GH746-REFRESH-ROUNDS-DO-NOT-PILE-UP: the save-driven dataRev bump waits
// for the refresh reads the native backend has in flight, so those reads must
// be counted where every one of them is issued, until each settles.
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke, convertFileSrc: (path: string) => path }));

afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); invoke.mockReset(); });

async function tauriBackend() {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  const { backend } = await import("./backend");
  const reads = await import("./refreshReads");
  const b = backend();
  await b.startupGraphPath(); // the native bridge has loaded
  return { b, reads };
}

it("counts a refresh read until it answers or fails, and not a page load", async () => {
  const finish = new Map<string, { resolve: (value: unknown) => void; reject: (error: Error) => void }>();
  invoke.mockImplementation((cmd: string) => cmd === "get_backlinks" || cmd === "resolve_blocks" || cmd === "get_page"
    ? new Promise((resolve, reject) => { finish.set(cmd, { resolve, reject }); })
    : Promise.resolve(null));
  const { b, reads } = await tauriBackend();
  const backlinks = b.getBacklinks("A");
  const refs = b.resolveBlocks(["u"]).catch(() => null);
  const page = b.getPage("A", "page");
  await vi.waitFor(() => expect(finish.size).toBe(3));
  expect(reads.refreshReadsInFlight()).toBe(2);
  let idle = false;
  void reads.refreshReadsIdle().then(() => { idle = true; });
  finish.get("get_backlinks")!.resolve([]);
  await backlinks;
  expect(reads.refreshReadsInFlight()).toBe(1);
  finish.get("resolve_blocks")!.reject(new Error("no graph"));
  await refs;
  await vi.waitFor(() => expect(idle).toBe(true));
  expect(reads.refreshReadsInFlight()).toBe(0);
  finish.get("get_page")!.resolve(null);
  await page;
});

it("every counted command is one the native backend issues", () => {
  const source = readFileSync(new URL("./backend.ts", import.meta.url), "utf8");
  for (const cmd of REFRESH_READ_COMMANDS) expect(source).toMatch(new RegExp(`this\\.call<.*?>\\("${cmd}"`));
});

it("counts the reads the block-referrer panel, the reference filter and All pages refetch after a save", async () => {
  // GX4 inventory: these readers refetch on dataRev (or on the refreshed groups) too.
  const pending: (() => void)[] = [];
  invoke.mockImplementation((cmd: string) => cmd === "startup_graph_path"
    ? Promise.resolve(null)
    : new Promise((resolve) => { pending.push(() => resolve(cmd === "page_modified" ? { modified: [] } : [])); }));
  const { b, reads } = await tauriBackend();
  const all = Promise.all([b.getBlockReferrers("u"), b.getBacklinkFilterContext("A", []), b.pageBacklinkCounts(), b.pageModified()]);
  await vi.waitFor(() => expect(pending.length).toBe(4));
  expect(reads.refreshReadsInFlight()).toBe(4);
  for (const finish of pending) finish();
  await all;
  expect(reads.refreshReadsInFlight()).toBe(0);
});
