// K22 (Martin, 2026-10-02; SPEC-storage §6.1): typing is never blocked by
// observation. The old freshness barrier cancelled keydown / beforeinput /
// compositionstart at the window for the whole focus-return rescan. A stale-base
// save is refused by the base-revision guard and becomes a conflict instead
// (crates/tine-store/src/graph_tests.rs `save_refuses_to_clobber_external_change`:
// typed doc + stale base rev -> SaveOutcome::Conflict, disk bytes untouched; and
// watch/rebuild.rs `rebuild_leaves_an_open_editors_base_revision_stale_not_overwritten`).
// This file is the DOM half: jsdom.
import { expect, it, vi } from "vitest";
import { backend } from "./backend";
import { installReloadOnFocus, refreshOnReturnToWindow } from "./reloadOnFocus";

type Api = ReturnType<typeof backend>;

it("keydown, beforeinput and composition during a focus-return rescan are not prevented", async () => {
  const api = backend() as Api;
  let complete: ((sequence: number) => void) | null = null;
  let rescans = 0;
  api.onGraphRescanComplete = async (cb) => { complete = cb; return () => {}; };
  api.rescanGraphNow = async () => { rescans++; return 7; };
  try {
    installReloadOnFocus();
    const types = ["keydown", "beforeinput", "compositionstart"];
    const reached: string[] = [];
    for (const type of types) document.addEventListener(type, () => reached.push(type));
    const refresh = refreshOnReturnToWindow(10_000);
    await vi.waitFor(() => expect(rescans).toBe(1)); // the rescan is in flight
    for (const type of types) {
      const event = new Event(type, { bubbles: true, cancelable: true });
      document.body.dispatchEvent(event);
      expect(event.defaultPrevented, `${type} must not be prevented`).toBe(false);
    }
    expect(reached).toEqual(types); // and none was stopped before reaching the page
    complete!(7);
    await refresh;
  } finally {
    delete api.onGraphRescanComplete;
    delete api.rescanGraphNow;
  }
});
