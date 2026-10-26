import { beforeAll, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { pageByName, setRaw, insertOutlineAfter, blockSubtreeMarkdown } from "./document";
import { CAPTURE_SCRATCH_NAME } from "./captureSeed";
import { initParser } from "./render/parse";

const h = vi.hoisted(() => ({
  api: null as null | { submit(): void; cancel(): void },
  listeners: new Map<string, (e: { payload: unknown }) => void>(),
  requests: [] as Array<{ id: string; text: string }>,
  hide: vi.fn(),
}));
vi.mock("./components/Block", () => ({ Block: () => null,
  CaptureCtx: { Provider: (p: { value: typeof h.api; children: unknown }) => { h.api = p.value; return p.children; } },
}));
vi.mock("./editorController", async (original) => ({ ...await original<typeof import("./editorController")>(), startEditing: () => {} }));
vi.mock("./keybindings", () => ({ installKeybindings: () => () => {}, eventToBindingString: () => "" }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, cb: (e: { payload: unknown }) => void) => {
    h.listeners.set(name, cb); return () => h.listeners.delete(name);
  },
  emitTo: async (_: string, event: string, payload: { id: string; text: string }) => {
    if (event === "quick-capture") h.requests.push(payload);
  },
}));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({
  label: "capture", isVisible: async () => false, onFocusChanged: async () => () => {}, hide: h.hide,
}) }));
beforeAll(async () => {
  await initParser();
  vi.spyOn(backend(), "bindCaptureGraph").mockImplementation(() => new Promise(() => {}));
  vi.spyOn(backend(), "getCaptureEnterFiles").mockResolvedValue(false);
  vi.spyOn(backend(), "captureTarget").mockResolvedValue("main");
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(17);
  const root = document.createElement("div"); root.id = "capture-root"; document.body.append(root);
  await import("./capture");
  await vi.waitFor(() => expect(h.api).not.toBeNull());
});
const roots = () => pageByName(CAPTURE_SCRATCH_NAME)!.roots;
const scratch = () => roots().map(id => blockSubtreeMarkdown(id)).join("\n");
async function submit() {
  const before = h.requests.length; h.api!.submit();
  await vi.waitFor(() => expect(h.requests.length).toBe(before + 1));
  return h.requests.at(-1)!;
}
async function ack(id: string, extra: object) {
  h.listeners.get("quick-capture-ack")!({ payload: { id, ...extra } });
  await Promise.resolve();
}

it("F3: retry saves the old snapshot once, and the next submit contains only later typing", async () => {
  setRaw(roots()[0], "old thought");
  const first = await submit();
  await ack(first.id, { ok: false, landed: true });
  setRaw(roots()[0], "old thought continued");
  insertOutlineAfter(roots()[0], [{ raw: "new thought", children: [] }]);
  const retry = await submit();
  expect(retry).toEqual(first);
  await ack(retry.id, { ok: true });
  expect(scratch()).not.toContain("old thought");
  expect(scratch()).toContain(" continued");
  expect(scratch()).toContain("new thought");
  const next = await submit();
  expect(next.id).not.toBe(first.id);
  expect(next.text).not.toContain("old thought");
  expect(first.text + next.text).toContain("new thought");
  await ack(next.id, { ok: true });
});

it("F3: a title-only edit does not retain already-filed text", async () => {
  setRaw(roots()[0], "saved under first title");
  const request = await submit();
  const title = document.querySelector<HTMLInputElement>(".capture-title")!;
  title.value = "Next destination"; title.dispatchEvent(new Event("input", { bubbles: true }));
  await ack(request.id, { ok: true });
  expect(scratch()).not.toContain("saved under first title");
  expect(title.value).toBe("Next destination");
  const hidden = h.hide.mock.calls.length;
  h.api!.submit();
  await Promise.resolve();
  expect(title.value).toBe("Next destination");
  expect(h.hide).toHaveBeenCalledTimes(hidden);
  expect(document.body.textContent).toContain("Add text");
  title.value = ""; title.dispatchEvent(new Event("input", { bubbles: true }));
});

it("F3: rewritten input is kept with an earlier-saved reminder", async () => {
  setRaw(roots()[0], "first version");
  const request = await submit();
  setRaw(roots()[0], "rewritten version");
  await ack(request.id, { ok: true });
  expect(scratch()).toContain("rewritten version");
  expect(document.body.textContent).toContain("Earlier capture saved");
  const rewritten = await submit();
  expect(rewritten.text).toContain("rewritten version");
  await ack(rewritten.id, { ok: true });
});

it("F3: an acknowledgement during IME waits for the composition commit before subtracting", async () => {
  setRaw(roots()[0], "before");
  const request = await submit();
  const shell = document.querySelector(".capture-shell")!;
  shell.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  await ack(request.id, { ok: true });
  expect(scratch()).toContain("before");
  setRaw(roots()[0], "before入力");
  shell.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
  await Promise.resolve();
  expect(scratch()).not.toContain("before");
  expect(scratch()).toContain("入力");
  const later = await submit();
  expect(later.text).toContain("入力");
  await ack(later.id, { ok: true });
});

it("F2: a closed-graph acknowledgement explains uncertainty and Cancel keeps the scratch", async () => {
  setRaw(roots()[0], "keep this thought");
  const request = await submit();
  await ack(request.id, { ok: false, landed: true, closedGraph: true });
  expect(document.body.textContent).toContain("previous graph");
  h.api!.cancel();
  await Promise.resolve();
  expect(scratch()).toContain("keep this thought");
  const next = document.querySelector<HTMLButtonElement>("[data-testid=capture-start-new]")!;
  expect(next).not.toBeNull();
  next.click();
  expect(scratch()).toContain("keep this thought");
  expect(document.body.textContent).toContain("already saved");
  // After checking the old graph the user can edit the retained text and file
  // a fresh snapshot, rather than remaining trapped on the retired request.
  setRaw(roots()[0], "only the still-missing thought");
  const fresh = await submit();
  expect(fresh.id).not.toBe(request.id);
  expect(fresh.text).toContain("only the still-missing thought");
  await ack(fresh.id, { ok: true });
});

it("FU2: cancelling before any acknowledgement keeps a plain retry", async () => {
  setRaw(roots()[0], "still waiting for acknowledgement");
  const request = await submit();
  h.api!.cancel();
  await Promise.resolve();
  expect(scratch()).toContain("still waiting for acknowledgement");
  expect(document.querySelector("[data-testid=capture-start-new]")).toBeNull();
  const retry = await submit();
  expect(retry.id).toBe(request.id);
  await ack(retry.id, { ok: true });
});

it("FU2: a same-graph timeout keeps the request identity without the previous-graph exit", async () => {
  setRaw(roots()[0], "slow live graph");
  vi.useFakeTimers();
  let request: { id: string; text: string };
  try {
    request = await submit();
    await vi.runAllTimersAsync();
    expect(scratch()).toContain("slow live graph");
    expect(document.querySelector("[data-testid=capture-start-new]")).toBeNull();
    expect(document.body.textContent).toContain("Retry File capture");
  } finally { vi.useRealTimers(); }
  const retried = await submit();
  expect(retried.id).toBe(request!.id);
  await ack(retried.id, { ok: true });
});

it("A1: the next File capture after an old-binding refusal refreshes the binding", async () => {
  setRaw(roots()[0], "retry in the current graph");
  const first = await submit();
  await ack(first.id, { ok: false });
  const bind = vi.spyOn(backend(), "bindCaptureGraph").mockImplementationOnce(async () => {
    vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(18);
  });
  const next = await submit();
  await ack(next.id, { ok: true });
  expect(bind).toHaveBeenCalled();
  expect(next.id).not.toBe(first.id);
  expect(next).toMatchObject({ bindingGeneration: 18 });
});

it("A1: a refused old binding keeps text on Esc and files next in the current binding", async () => {
  setRaw(roots()[0], "refused but still needed");
  const first = await submit();
  await ack(first.id, { ok: false });
  h.api!.cancel();
  await Promise.resolve();
  expect(scratch()).toContain("refused but still needed");
  const bind = vi.spyOn(backend(), "bindCaptureGraph").mockImplementationOnce(async () => {
    vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(19);
  });
  const next = await submit();
  expect(bind).toHaveBeenCalled();
  expect(next.id).not.toBe(first.id);
  expect(next).toMatchObject({ bindingGeneration: 19 });
  await ack(next.id, { ok: true });
});
