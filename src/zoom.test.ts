import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { toasts, setToasts } from "./toasts";
import { MAIN_WINDOW_ID, registerWindow } from "./windowRealm";
import {
  ZOOM_WHEEL_MOMENTUM_TAIL_MS,
  decideWheelZoomGesture,
  interfaceZoom,
  zoomIn,
  installInterfaceZoomWheel,
  zoomReset,
  type WheelZoomGestureState,
} from "./zoom";

type FakeListener = {
  type: string;
  listener: EventListener;
  capture: boolean;
};

let restoreFakeGlobals: (() => void) | null = null;
let uninstallWheel: (() => void) | null = null;

function installFakeWheelGlobals() {
  const listeners: FakeListener[] = [];
  const rafs = new Map<number, FrameRequestCallback>();
  let nextRaf = 1;
  const windowDescriptor = Object.getOwnPropertyDescriptor(globalThis, "window");
  const rafDescriptor = Object.getOwnPropertyDescriptor(globalThis, "requestAnimationFrame");
  const cancelRafDescriptor = Object.getOwnPropertyDescriptor(globalThis, "cancelAnimationFrame");
  const fakeWindow = {
    addEventListener(type: string, listener: EventListener, options?: boolean | AddEventListenerOptions) {
      listeners.push({
        type,
        listener,
        capture: options === true || !!(options as AddEventListenerOptions | undefined)?.capture,
      });
    },
    removeEventListener(type: string, listener: EventListener, options?: boolean | EventListenerOptions) {
      const capture = options === true || !!(options as EventListenerOptions | undefined)?.capture;
      const i = listeners.findIndex((l) => l.type === type && l.listener === listener && l.capture === capture);
      if (i >= 0) listeners.splice(i, 1);
    },
  };

  const requestFakeFrame = (cb: FrameRequestCallback) => {
    const id = nextRaf++;
    rafs.set(id, cb);
    return id;
  };
  const cancelFakeFrame = (id: number) => {
    rafs.delete(id);
  };
  // A window owns its frames and document and refers to itself, as a real one does.
  Object.assign(fakeWindow, {
    window: fakeWindow,
    document: {},
    requestAnimationFrame: requestFakeFrame,
    cancelAnimationFrame: cancelFakeFrame,
  });

  Object.defineProperty(globalThis, "window", { value: fakeWindow, configurable: true });
  Object.defineProperty(globalThis, "requestAnimationFrame", { value: requestFakeFrame, configurable: true });
  Object.defineProperty(globalThis, "cancelAnimationFrame", { value: cancelFakeFrame, configurable: true });

  // The app registers its real main window at load (src/windowRealm.ts); this
  // node-pool fake stands in for it so the wheel hook's onEachWindow reaches it.
  const unregisterFake = registerWindow(MAIN_WINDOW_ID, fakeWindow as unknown as Window);

  restoreFakeGlobals = () => {
    unregisterFake();
    if (windowDescriptor) Object.defineProperty(globalThis, "window", windowDescriptor);
    else delete (globalThis as { window?: Window }).window;
    if (rafDescriptor) Object.defineProperty(globalThis, "requestAnimationFrame", rafDescriptor);
    else delete (globalThis as { requestAnimationFrame?: typeof requestAnimationFrame }).requestAnimationFrame;
    if (cancelRafDescriptor) Object.defineProperty(globalThis, "cancelAnimationFrame", cancelRafDescriptor);
    else delete (globalThis as { cancelAnimationFrame?: typeof cancelAnimationFrame }).cancelAnimationFrame;
    restoreFakeGlobals = null;
  };

  return {
    dispatchCaptureWheel(event: WheelEvent) {
      listeners
        .filter((l) => l.type === "wheel" && l.capture)
        .forEach((l) => l.listener(event));
    },
    flushAnimationFrames() {
      const pending = Array.from(rafs.values());
      rafs.clear();
      pending.forEach((cb) => cb(0));
    },
  };
}

function wheelEvent(init: Partial<WheelEvent> & { timeStamp: number }) {
  let prevented = false;
  let stopped = false;
  const event = {
    ctrlKey: false,
    metaKey: false,
    deltaY: 0,
    target: { closest: () => null },
    preventDefault: () => {
      prevented = true;
    },
    stopPropagation: () => {
      stopped = true;
    },
    ...init,
  } as unknown as WheelEvent;

  return {
    event,
    prevented: () => prevented,
    stopped: () => stopped,
  };
}

beforeEach(() => {
  zoomReset();
});

afterEach(() => {
  uninstallWheel?.();
  uninstallWheel = null;
  restoreFakeGlobals?.();
  zoomReset();
});

describe("wheel zoom momentum guard", () => {
  it("classifies a modified wheel after recent plain wheel activity as consume-only", () => {
    let state: WheelZoomGestureState = {};

    let decision = decideWheelZoomGesture(state, false, 1000);
    state = decision.state;
    expect(decision).toMatchObject({ consume: false, zoom: false });

    decision = decideWheelZoomGesture(state, true, 1000 + ZOOM_WHEEL_MOMENTUM_TAIL_MS / 2);
    expect(decision).toMatchObject({ consume: true, zoom: false });
  });

  it("allows modifier-first and after-idle wheel gestures to zoom", () => {
    const modifierFirst = decideWheelZoomGesture({}, true, 1000);
    expect(modifierFirst).toMatchObject({ consume: true, zoom: true });

    const plain = decideWheelZoomGesture({}, false, 1000);
    const afterIdle = decideWheelZoomGesture(plain.state, true, 1000 + ZOOM_WHEEL_MOMENTUM_TAIL_MS + 1);
    expect(afterIdle).toMatchObject({ consume: true, zoom: true });
  });
});

describe("installInterfaceZoomWheel", () => {
  it("consumes a modified momentum tail without applying interface zoom", () => {
    const fake = installFakeWheelGlobals();
    uninstallWheel = installInterfaceZoomWheel();

    const plain = wheelEvent({ timeStamp: 1000, deltaY: 120 });
    fake.dispatchCaptureWheel(plain.event);
    expect(plain.prevented()).toBe(false);
    expect(plain.stopped()).toBe(false);

    const tail = wheelEvent({
      timeStamp: 1000 + ZOOM_WHEEL_MOMENTUM_TAIL_MS / 2,
      metaKey: true,
      deltaY: -120,
    });
    fake.dispatchCaptureWheel(tail.event);
    fake.flushAnimationFrames();

    expect(tail.prevented()).toBe(true);
    expect(tail.stopped()).toBe(true);
    expect(interfaceZoom()).toBe(1);
  });

  it("applies interface zoom for a modifier-first wheel gesture", () => {
    const fake = installFakeWheelGlobals();
    uninstallWheel = installInterfaceZoomWheel();

    const wheel = wheelEvent({ timeStamp: 1000, metaKey: true, deltaY: -120 });
    fake.dispatchCaptureWheel(wheel.event);
    fake.flushAnimationFrames();

    expect(wheel.prevented()).toBe(true);
    expect(wheel.stopped()).toBe(true);
    expect(interfaceZoom()).toBe(1.1);
  });

  it("applies interface zoom after the plain-scroll momentum window has idled", () => {
    const fake = installFakeWheelGlobals();
    uninstallWheel = installInterfaceZoomWheel();

    fake.dispatchCaptureWheel(wheelEvent({ timeStamp: 1000, deltaY: 120 }).event);
    const wheel = wheelEvent({
      timeStamp: 1000 + ZOOM_WHEEL_MOMENTUM_TAIL_MS + 1,
      metaKey: true,
      deltaY: -120,
    });
    fake.dispatchCaptureWheel(wheel.event);
    fake.flushAnimationFrames();

    expect(wheel.prevented()).toBe(true);
    expect(wheel.stopped()).toBe(true);
    expect(interfaceZoom()).toBe(1.1);
  });
});
it("reports a failed interface zoom storage write", () => {
  const old = interfaceZoom();
  vi.stubGlobal("localStorage", { setItem: () => { throw new Error("quota"); }, removeItem: () => { throw new Error("quota"); } });
  zoomIn();
  expect(interfaceZoom()).toBeGreaterThan(old);
  expect(toasts().some((toast) => toast.kind === "error")).toBe(true);
  vi.unstubAllGlobals();
  zoomReset();
  setToasts([]);
});
