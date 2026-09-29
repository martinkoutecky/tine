import { afterEach, expect, it, vi } from "vitest";

// GH #343: a slow or failed command reaches the flight recorder as its
// registered name, a fixed phase and a duration — never its arguments.
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke, convertFileSrc: (path: string) => path }));

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); vi.resetModules(); invoke.mockReset(); });

async function tauriBackend() {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  const { backend } = await import("./backend");
  return backend();
}

const phases = () => invoke.mock.calls.filter(([cmd]) => cmd === "diagnostic_ipc_event").map(([, args]) => args);

it("records a slow command when it crosses the threshold and again when it completes", async () => {
  let finish!: (value: unknown) => void;
  invoke.mockImplementation((cmd: string) => cmd === "inspect_graph_access"
    ? new Promise((resolve) => { finish = resolve; })
    : Promise.resolve(null));
  const backend = await tauriBackend();
  await backend.startupGraphPath(); // the native bridge has loaded
  vi.useFakeTimers();
  const pending = backend.inspectGraphAccess("/home/someone/secret");
  await vi.advanceTimersByTimeAsync(600);
  expect(phases()).toEqual([{ command: "inspect_graph_access", phase: "slow", elapsedMs: expect.any(Number) }]);
  finish(null);
  await pending;
  expect(phases().map((args) => (args as { phase: string }).phase)).toEqual(["slow", "completed"]);
  expect(JSON.stringify(phases())).not.toContain("secret");
});

it("records a failed command and still rejects to the caller", async () => {
  invoke.mockImplementation((cmd: string) => cmd === "diagnostic_ipc_event"
    ? Promise.resolve()
    : Promise.reject(new Error("io:/home/someone/graph")));
  const backend = await tauriBackend();
  await expect(backend.startupGraphPath()).rejects.toThrow("io:/home/someone/graph");
  expect(phases()).toEqual([{ command: "startup_graph_path", phase: "failed", elapsedMs: expect.any(Number) }]);
});

it("does not time the diagnostics channel itself, and stops reporting once the recorder refuses", async () => {
  invoke.mockImplementation((cmd: string) => cmd === "diagnostic_ipc_event"
    ? Promise.reject(new Error("command not found"))
    : Promise.reject(new Error("boom")));
  const backend = await tauriBackend();
  await expect(backend.diagnosticReport("", "")).rejects.toThrow("boom");
  expect(phases()).toEqual([]);
  await expect(backend.startupGraphPath()).rejects.toThrow("boom");
  await Promise.resolve();
  await expect(backend.startupGraphPath()).rejects.toThrow("boom");
  expect(phases()).toHaveLength(1);
});
