// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { IME_TRACE_TAIL_MS, IME_TRACE_TIMEOUT_MS, recordImeTrace } from "./imeTrace";

// GH #767: the opt-in Enter trace records no typed text, and an armed trace ends by itself.
beforeEach(() => { vi.useFakeTimers(); });
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

const key = (value: string, type = "keydown") =>
  document.body.dispatchEvent(new KeyboardEvent(type, { key: value, bubbles: true }));

it.each([
  ["an emoji", "😀"],
  ["a combining sequence", "é"],
  ["a CJK character", "你"],
])("records %s typed as a key as \"char\", never its text", async (_name, typed) => {
  const trace = recordImeTrace();
  key(typed); key(typed, "keyup");
  document.body.dispatchEvent(new InputEvent("beforeinput", { inputType: "insertText", data: typed, bubbles: true }));
  key("Enter");
  await vi.advanceTimersByTimeAsync(IME_TRACE_TAIL_MS);
  const text = await trace;
  expect(text).toContain("keydown key=char");
  expect(text).toContain("beforeinput inputType=insertText isComposing=false dataLength=" + typed.length);
  expect(text).toContain("keydown key=Enter");
  expect(text).not.toContain(typed);
  expect(text).toMatch(/stopped=enter/);
});

it("records named keys by name", async () => {
  const trace = recordImeTrace();
  for (const name of ["Backspace", "Unidentified", "Process", "ArrowLeft", "Dead", "Shift"]) key(name);
  key("Enter");
  await vi.advanceTimersByTimeAsync(IME_TRACE_TAIL_MS);
  const text = await trace;
  for (const name of ["Backspace", "Unidentified", "Process", "ArrowLeft", "Dead", "Shift"]) expect(text).toContain(`key=${name} `);
});

it("an armed trace nobody presses Enter in stops by itself and removes every listener it added", async () => {
  const add = vi.spyOn(document, "addEventListener"), remove = vi.spyOn(document, "removeEventListener");
  let settled = false;
  const trace = recordImeTrace().then(text => { settled = true; return text; });
  const added = add.mock.calls.length;
  expect(added).toBeGreaterThan(0);
  key("a");
  await vi.advanceTimersByTimeAsync(IME_TRACE_TIMEOUT_MS - 1);
  expect(settled).toBe(false);
  await vi.advanceTimersByTimeAsync(1);
  const text = await trace;
  expect(text).toMatch(/stopped=timeout/);
  expect(remove.mock.calls.map(c => [c[0], c[1], c[2]])).toEqual(add.mock.calls.map(c => [c[0], c[1], c[2]]));
  key("b");
  expect(vi.getTimerCount()).toBe(0);
});
