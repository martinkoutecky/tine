import { expect, it, vi } from "vitest";
import { installMouseHistory } from "./mouseHistory";

function fixture(source: "dom" | "gtk" | "webview2" = "dom") {
  const target = new EventTarget();
  const doc = new EventTarget();
  const navigate = vi.fn();
  const dispose = installMouseHistory(target as Window, doc as Document, navigate, source);
  const send = (type: string, values: Record<string, unknown> = {}) => {
    const event = new Event(type, { cancelable: true });
    Object.assign(event, { button: 3, buttons: 0, clientX: 20, clientY: 20, ...values });
    target.dispatchEvent(event);
    return event;
  };
  return { navigate, dispose, send, doc };
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
