import { expect, it, vi } from "vitest";
import { installMouseHistory } from "./mouseHistory";

function fixture(source: "dom" | "gtk" | "webview2" = "dom") {
  const target = new EventTarget();
  const doc = new EventTarget();
  const navigate = vi.fn();
  const listeners = new Map<string, (event: Event) => void>();
  const add = target.addEventListener.bind(target);
  target.addEventListener = ((name: string, listener: EventListener, options?: AddEventListenerOptions | boolean) => {
    listeners.set(name, listener);
    add(name, listener, options);
  }) as typeof target.addEventListener;
  const dispose = installMouseHistory(target as Window, doc as Document, navigate, source);
  // Blink gives an auxclick its release's timestamp; `fresh: true` sends an auxclick of a release the page never saw.
  let clock = 0;
  const lastUp = new Map<unknown, number>();
  const send = (type: string, { fresh, ...values }: Record<string, unknown> = {}) => {
    const event = new Event(type, { cancelable: true });
    Object.assign(event, { button: 3, buttons: 0, clientX: 20, clientY: 20, ...values });
    const button = (event as MouseEvent).button;
    const stamp = type === "auxclick" && !fresh && lastUp.has(button) ? lastUp.get(button)! : ++clock;
    if (type === "mouseup") lastUp.set(button, stamp);
    Object.defineProperty(event, "timeStamp", { value: stamp });
    target.dispatchEvent(event);
    return event;
  };
  return { navigate, dispose, send, doc, listeners };
}

it("routes Wry's macOS down/up pair once and consumes the browser default", () => {
  const f = fixture();
  f.send("mousedown", { buttons: 8 });
  expect(f.navigate).not.toHaveBeenCalled();
  expect(f.send("mouseup").defaultPrevented).toBe(true);
  f.send("auxclick");
  expect(f.navigate.mock.calls).toEqual([["back"]]);
  f.send("mousedown", { button: 4, buttons: 16 });
  f.send("mouseup", { button: 4 });
  expect(f.navigate.mock.calls).toEqual([["back"], ["forward"]]);
  f.dispose();
  f.send("auxclick");
  expect(f.navigate).toHaveBeenCalledTimes(2);
});

it.each(["dragstart", "mousemove", "selectionchange"])("does not navigate after %s uses a side button", (type) => {
  const f = fixture();
  f.send("mousedown", { buttons: 8 });
  if (type === "selectionchange") f.doc.dispatchEvent(new Event(type));
  else f.send(type, { buttons: 8, clientX: 50 });
  f.send("mouseup");
  f.send("auxclick");
  expect(f.navigate).not.toHaveBeenCalled();
  f.send("dragend");
  f.send("mousedown", { buttons: 8 });
  f.send("mouseup");
  expect(f.navigate.mock.calls).toEqual([["back"]]);
  f.dispose();
});

it("supports auxclick-only delivery and leaves middle click alone", () => {
  const f = fixture();
  expect(f.send("auxclick", { button: 1 }).defaultPrevented).toBe(false);
  f.send("auxclick");
  f.send("auxclick");
  expect(f.navigate.mock.calls).toEqual([["back"], ["back"]]);
  f.dispose();
});

it("rejects a moved macOS release even when no DOM mousemove was delivered", () => {
  const f = fixture();
  f.send("mousedown", { buttons: 8 });
  f.send("mouseup", { clientX: 60 });
  f.send("auxclick");
  expect(f.navigate).not.toHaveBeenCalled();
  f.send("mousedown", { buttons: 8 });
  f.send("mouseup");
  expect(f.navigate.mock.calls).toEqual([["back"]]);
  f.dispose();
});

it("does not navigate while another button owns a text selection, then accepts the next click", () => {
  const f = fixture();
  f.send("mousedown", { button: 0, buttons: 1 });
  f.send("mousedown", { buttons: 9 });
  f.send("mouseup", { buttons: 1 });
  f.send("auxclick", { buttons: 1 });
  expect(f.navigate).not.toHaveBeenCalled();
  f.send("mouseup", { button: 0 });
  f.send("mousedown", { buttons: 8 });
  f.send("mouseup");
  expect(f.navigate.mock.calls).toEqual([["back"]]);
  f.dispose();
});

// GH #948 review: two side buttons held at once keep separate gestures.
it.each([["dom", true], ["dom", false], ["webview2", true], ["webview2", false]] as const)(
  "%s overlapping Back/Forward presses each step once in both release orders (auxclick %s)", (source, aux) => {
  for (const olderFirst of [true, false]) {
    const f = fixture(source);
    f.send("mousedown", { button: 3, buttons: 8 });
    f.send("mousedown", { button: 4, buttons: 24 });
    const order = olderFirst ? [3, 4] : [4, 3];
    for (const button of order) {
      f.send("mouseup", { button });
      if (aux) f.send("auxclick", { button });
    }
    expect(f.navigate.mock.calls).toEqual(order.map((b) => [b === 3 ? "back" : "forward"]));
    f.dispose();
  }
});

it.each(["dom", "webview2"] as const)("%s overlapping cancelled presses never navigate, in either release order", (source) => {
  for (const olderFirst of [true, false]) {
    const f = fixture(source);
    f.send("mousedown", { button: 3, buttons: 8 });
    f.send("mousedown", { button: 4, buttons: 24 });
    f.send("mousemove", { buttons: 24, clientX: 60 });
    for (const button of olderFirst ? [3, 4] : [4, 3]) {
      f.send("mouseup", { button, clientX: 60 });
      f.send("auxclick", { button, clientX: 60 });
    }
    expect(f.navigate).not.toHaveBeenCalled();
    f.dispose();
  }
});

// GH #948 review r2 F3: a release marker never outlives the next press, whatever button it is.
it.each(["dom", "webview2"] as const)("%s auxclick-only presses after an interleaved click still step", (source) => {
  for (const between of [4, 0]) {
    const f = fixture(source);
    f.send("mousedown", { button: 3, buttons: 8 });
    f.send("mouseup", { button: 3 });
    // No auxclick for that release; another button is clicked, then Back arrives as auxclick only.
    f.send("mousedown", { button: between, buttons: between === 4 ? 16 : 1 });
    f.send("mouseup", { button: between });
    f.send("auxclick", { button: 3 });
    expect(f.navigate.mock.calls.filter(([d]) => d === "back")).toHaveLength(2);
    f.dispose();
  }
});

// GH #948 review (1dfbea5f2) F1: a release without its auxclick never eats a later auxclick-only press.
it.each(["dom", "webview2"] as const)("%s auxclick-only press after an unpaired release steps, both buttons", (source) => {
  for (const button of [3, 4]) {
    const f = fixture(source);
    f.send("mousedown", { button, buttons: button === 3 ? 8 : 16 });
    f.send("mouseup", { button });
    f.send("mousemove", { buttons: 0, clientX: 40 });
    f.send("auxclick", { button, fresh: true });
    expect(f.navigate).toHaveBeenCalledTimes(2);
    f.dispose();
  }
});

// F2: releasing one ordinary button while another stays down keeps the gesture occupied.
it.each(["dom", "webview2"] as const)("%s side clicks wait while another ordinary button is still held", (source) => {
  const f = fixture(source);
  f.send("mousedown", { button: 0, buttons: 1 });
  f.send("mousedown", { button: 2, buttons: 3 });
  f.send("mouseup", { button: 0, buttons: 2 });
  f.send("mousedown", { button: 3, buttons: 10 });
  f.send("mouseup", { button: 3, buttons: 2 });
  f.send("auxclick", { button: 3, buttons: 2 });
  f.send("auxclick", { button: 4, buttons: 2, fresh: true });
  expect(f.navigate).not.toHaveBeenCalled();
  f.dispose();
});

// GH #948 review (187d3d4d3) F2: an editor blurring inside the window keeps the gesture;
// the window itself losing focus still ends it.
it.each(["dom", "webview2"] as const)("%s element blur inside the window keeps side-button gestures", (source) => {
  for (const button of [3, 4]) {
    const f = fixture(source);
    const blur = f.listeners.get("blur")!;
    const inner = () => blur({ type: "blur", target: new EventTarget() } as unknown as Event);
    const down = () => f.send("mousedown", { button, buttons: button === 3 ? 8 : 16 });
    // Release-only delivery (no auxclick) with a blur between down and up.
    down(); inner(); f.send("mouseup", { button });
    expect(f.navigate).toHaveBeenCalledTimes(1);
    // Paired delivery with a blur between up and its auxclick: one step, not two.
    f.send("mousemove", { clientX: 20 });
    down(); f.send("mouseup", { button }); inner(); f.send("auxclick", { button });
    expect(f.navigate).toHaveBeenCalledTimes(2);
    f.dispose();
  }
});

it("a window blur still ends a pending side-button press", () => {
  const f = fixture("dom");
  f.send("mousedown", { buttons: 8 });
  f.send("blur");
  f.send("auxclick", { fresh: true });
  f.send("mouseup");
  expect(f.navigate).toHaveBeenCalledTimes(1);
  f.dispose();
});
