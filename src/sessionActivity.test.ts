import { describe, it, expect } from "vitest";
import { installSessionActivity } from "./sessionActivity";

function harness(opts: { isMobile: boolean }) {
  const listeners = new Map<string, Set<EventListenerOrEventListenerObject>>();
  const calls: boolean[] = [];
  let hidden = false;
  const uninstall = installSessionActivity({
    isMobile: opts.isMobile,
    setActive: (active) => calls.push(active),
    isHidden: () => hidden,
    addEventListener: ((type: string, fn: EventListenerOrEventListenerObject) => {
      if (!listeners.has(type)) listeners.set(type, new Set());
      listeners.get(type)!.add(fn);
    }) as typeof document.addEventListener,
    removeEventListener: ((type: string, fn: EventListenerOrEventListenerObject) => {
      listeners.get(type)?.delete(fn);
    }) as typeof document.removeEventListener,
  });
  const fire = (type: string) => {
    for (const fn of listeners.get(type) ?? []) {
      if (typeof fn === "function") fn(new Event(type));
      else fn.handleEvent(new Event(type));
    }
  };
  return {
    calls,
    fire,
    uninstall,
    listenerCount: () => [...listeners.values()].reduce((n, set) => n + set.size, 0),
    hide: () => { hidden = true; },
    show: () => { hidden = false; },
  };
}

describe("recorded session lifetime (GH #426)", () => {
  it("ends the session when a mobile app is backgrounded and restarts it on return", () => {
    const h = harness({ isMobile: true });
    h.hide();
    h.fire("visibilitychange");
    expect(h.calls).toEqual([false]);
    h.show();
    h.fire("visibilitychange");
    expect(h.calls).toEqual([false, true]);
  });

  it("accepts pagehide as the end, since a WebView may deliver only that", () => {
    const h = harness({ isMobile: true });
    // Deliberately leave visibilityState reading "visible": iOS has been seen
    // to deliver pagehide before the document is marked hidden.
    h.fire("pagehide");
    expect(h.calls).toEqual([false]);
    h.fire("pageshow");
    expect(h.calls).toEqual([false, true]);
  });

  it("reports each edge once however many events one background delivers", () => {
    const h = harness({ isMobile: true });
    h.hide();
    h.fire("visibilitychange");
    h.fire("pagehide");
    h.fire("freeze");
    expect(h.calls).toEqual([false]);
  });

  // A minimised or occluded desktop window is still a running session, and a
  // crash behind it is exactly what the unclean-exit warning is for. If this
  // ever starts calling the backend, desktop stops reporting those crashes.
  it("never ends a desktop session on visibility", () => {
    const h = harness({ isMobile: false });
    h.hide();
    h.fire("visibilitychange");
    h.fire("pagehide");
    expect(h.calls).toEqual([]);
    expect(h.listenerCount()).toBe(0);
  });

  it("stops listening once uninstalled", () => {
    const h = harness({ isMobile: true });
    h.uninstall();
    expect(h.listenerCount()).toBe(0);
  });
});
