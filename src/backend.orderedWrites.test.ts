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
