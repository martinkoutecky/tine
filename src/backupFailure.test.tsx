import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { installPaneTracker, setGraphTransitioning } from "./ui";
import { toasts, setToasts } from "./toasts";

const h = vi.hoisted(() => ({ listener: null as null | ((event: { payload: { bindingGeneration: number; failure: string } }) => void), stop: vi.fn() }));
vi.mock("@tauri-apps/api/core", async (original) => ({ ...await original<typeof import("@tauri-apps/api/core")>(), isTauri: () => true }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async (_name: string, listener: typeof h.listener) => { h.listener = listener; return h.stop; } }));
afterEach(() => { vi.restoreAllMocks(); setToasts([]); setGraphTransitioning(false); });

it("reports a current launch backup failure and ignores retired graph failures", async () => {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(17);
  const stop = installPaneTracker();
  await vi.waitFor(() => expect(h.listener).not.toBeNull());
  h.listener!({ payload: { bindingGeneration: 16, failure: "backup-failed:pages:PermissionDenied" } });
  expect(toasts()).toEqual([]);
  h.listener!({ payload: { bindingGeneration: 17, failure: "backup-failed:pages:PermissionDenied" } });
  expect(toasts()).toHaveLength(1);
  expect(toasts()[0]).toMatchObject({ kind: "error", sticky: true });
  stop();
  await vi.waitFor(() => expect(h.stop).toHaveBeenCalledOnce());
});

it("keeps an early backup failure until the load reply publishes the owning binding", async () => {
  const generation = vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(17);
  setGraphTransitioning(true);
  const stop = installPaneTracker();
  await Promise.resolve();
  h.listener!({ payload: { bindingGeneration: 18, failure: "backup-failed:source:Other" } });
  h.listener!({ payload: { bindingGeneration: 19, failure: "backup-failed:pages:PermissionDenied" } });
  expect(toasts()).toEqual([]);
  generation.mockReturnValue(18);
  setGraphTransitioning(false);
  await vi.waitFor(() => expect(toasts()).toHaveLength(1));
  expect(toasts()[0]).toMatchObject({ kind: "error", sticky: true });
  stop();
});
