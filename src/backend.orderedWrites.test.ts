import { afterEach, expect, it, vi } from "vitest";

// R3 (og-flow3 finding 3): write commands now run on the native blocking pool,
// where two concurrent calls are not ordered. The frontend issues each
// ORDERED_COMMANDS call only after the previous one settled, so a page submitted
// and then closed is closed after its submit, and a setter toggled twice ends at
// the second value, exactly as when the main thread ran them one at a time.
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke, convertFileSrc: (path: string) => path }));

afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); invoke.mockReset(); });

async function tauriBackend() {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  const { backend } = await import("./backend");
  const b = backend();
  await b.startupGraphPath(); // the native bridge has loaded
  return b;
}

const issued = () => invoke.mock.calls.map(([cmd]) => cmd as string).filter((cmd) => cmd !== "diagnostic_ipc_event");

it("issues the second ordered write only after the first one settled", async () => {
  const finish: Array<(value: unknown) => void> = [];
  invoke.mockImplementation((cmd: string) => cmd === "page_submit" || cmd === "page_close"
    ? new Promise((resolve) => { finish.push(resolve); })
    : Promise.resolve(null));
  const b = await tauriBackend();
  const pageCommands = () => issued().filter((cmd) => cmd === "page_submit" || cmd === "page_close");
  const submitted = b.pageSubmit(1, 1, "pages/A.md", { name: "A" } as never, 1, null, ["edit-block"] as never);
  const closed = b.pageClose(1, 2, "pages/A.md");
  await vi.waitFor(() => expect(issued()).toContain("page_submit"));
  // Let the close's issue run if the lane failed to hold it (a macrotask, not a timed wait).
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(pageCommands()).toEqual(["page_submit"]);
  finish[0](null);
  await submitted;
  await vi.waitFor(() => expect(pageCommands()).toEqual(["page_submit", "page_close"]));
  finish[1](null);
  await closed;
});

it("a failed ordered write rejects only its own call and does not jam the lane", async () => {
  invoke.mockImplementation((cmd: string, args?: { value?: boolean }) => cmd === "set_smooth_scroll" && args?.value === true
    ? Promise.reject(new Error("disk full"))
    : Promise.resolve(null));
  const b = await tauriBackend();
  const first = b.setSmoothScroll(true);
  const second = b.setSmoothScroll(false);
  await expect(first).rejects.toThrow("disk full");
  await expect(second).resolves.toBeNull();
  const values = invoke.mock.calls.filter(([cmd]) => cmd === "set_smooth_scroll").map(([, args]) => (args as { value: boolean }).value);
  expect(values).toEqual([true, false]);
});

it("does not hold a read behind a pending ordered write", async () => {
  invoke.mockImplementation((cmd: string) => cmd === "page_submit" ? new Promise(() => {}) : Promise.resolve(null));
  const b = await tauriBackend();
  void b.pageSubmit(1, 1, "pages/A.md", { name: "A" } as never, 1, null, ["edit-block"] as never);
  await expect(b.startupGraphPath()).resolves.toBeNull();
});

// PERF-FIXES D3: a carry-over opens its source pages back to back
// (client.ts `open` does not await), and they reach the host as one
// `page_open` call: one IPC and one writer section instead of one per page.
// A name that resolves to an alias is answered without being sent.
it("joins back-to-back page opens into one page_open call", async () => {
  invoke.mockImplementation((cmd: string, args?: { pages?: Array<{ id: number; path: string }> }) => {
    if (cmd === "resolve_page") return Promise.resolve({ kind: "alias", owners: ["pages/Owner.md"] });
    if (cmd === "page_open") return Promise.resolve(args!.pages!.map(({ path }) => ({ key: path, baselineEntry: true })));
    return Promise.resolve(null);
  });
  const b = await tauriBackend();
  const opened = [1, 2, 3, 4, 5].map((id) => b.pageOpen(1, id, { name: `P${id}`, kind: "page", path: `pages/P${id}.md` }));
  const alias = b.pageOpen(1, 6, { name: "Alias", kind: "page", path: null });
  expect(await Promise.all(opened)).toEqual([1, 2, 3, 4, 5].map((id) => ({ key: `pages/P${id}.md`, baselineEntry: true })));
  expect(await alias).toEqual({ reason: "alias", owners: ["pages/Owner.md"] });
  const opens = invoke.mock.calls.filter(([cmd]) => cmd === "page_open");
  expect(opens).toHaveLength(1);
  expect((opens[0][1] as { pages: Array<{ id: number }> }).pages.map(({ id }) => id)).toEqual([1, 2, 3, 4, 5]);
});
