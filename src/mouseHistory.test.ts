import { expect, it, vi } from "vitest";
import { installMouseHistory } from "./mouseHistory";

function fixture() {
  const target = new EventTarget();
  const doc = new EventTarget();
  const navigate = vi.fn();
  const dispose = installMouseHistory(target as Window, doc as Document, navigate);
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
  expect(f.dispose.nativeAllowed()).toBe(false); // delayed native release too
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
  expect(f.navigate).not.toHaveBeenCalled();
  expect(f.dispose.nativeAllowed()).toBe(false);
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
