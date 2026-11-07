import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "./backend";
import { editorCompletions, type CompletionAdapter } from "./editor/editorCompletions";
import { QuickSwitcher } from "./components/QuickSwitcher";
import { loadSingle } from "./document/workingSet";
import { resetStore } from "./document";
import { openSwitcher, closeSwitcher } from "./ui";
import { invalidateBinding } from "./binding";
import type { PageEntry, QueryExecution } from "./types";

// Work/wait budgets, not host wall time. This provider represents the complete
// 10k name pool; each invocation counts one native rank of that pool. The real
// release-app journey measures ranking and paint on the default realistic graph.
const names: PageEntry[] = Array.from({ length: 10_000 }, (_, i) => ({
  name: `Topic ${i}`, kind: "page", path: `pages/Topic ${i}.md`, date_key: null,
}));
const result = (query: string) => names.filter(page => page.name.toLowerCase().includes(query.toLowerCase())).slice(0, 100);
const flush = async () => { for (let turn = 0; turn < 30; turn++) await Promise.resolve(); };
const disposals: (() => void)[] = [];

beforeEach(() => {
  vi.useFakeTimers();
  loadSingle({ name: "Source", title: "Source", kind: "page", pre_block: null,
    blocks: [{ id: "source", raw: "baseline [[Keep]]", collapsed: false, children: [] }] });
});
afterEach(() => {
  disposals.splice(0).forEach(dispose => dispose());
  closeSwitcher(); resetStore(); document.body.innerHTML = "";
  vi.useRealTimers(); vi.restoreAllMocks();
});

function editor(capture = false) {
  let raw = "", current = true;
  const adapter: CompletionAdapter = {
    id: "source", format: () => "md", current: () => current, composing: () => false,
    suppressTriggers: () => false, text: () => raw, selection: () => [raw.length, raw.length],
    caret: () => raw.length, rawBeforeCommit: () => "baseline [[Keep]]", visible: () => raw,
    apply: vi.fn(), replace: vi.fn(), afterEdit: callback => callback(), languagePicker: vi.fn(),
    closed: vi.fn(), element: () => document.createElement("textarea"), resultTarget: () => undefined,
    openJournal: vi.fn(), assets: { uploadAsset: vi.fn(), desktopVoiceMemoToggle: vi.fn(), createDrawioDiagram: vi.fn() },
    ...(capture ? { quickSwitch: (q: string, limit: number) => backend().captureQuickSwitch(q, limit) } : {}),
  };
  const core = createRoot(dispose => { disposals.push(dispose); return editorCompletions(adapter); });
  return { core, input: (value: string) => { raw = value; core.refresh(); }, retire: () => { current = false; } };
}

describe("GH #899: completion responds during typing on the 10k name pool", () => {
  it.each([["[[", false], ["#", false], ["[[", true]] as const)("%s (Capture=%s) has no typing-pause wait", async (trigger, capture) => {
    const implementation = async (q: string) => result(q);
    const read = capture ? vi.spyOn(backend(), "captureQuickSwitch").mockImplementation(implementation)
      : vi.spyOn(backend(), "quickSwitch").mockImplementation(implementation);
    const mounted = editor(capture);
    mounted.input(trigger); await flush();
    expect(read).not.toHaveBeenCalled(); // OG's blank opener has no rows.
    for (const q of ["t", "to", "top", "topi"]) {
      mounted.input(trigger + q); await flush();
      expect(read.mock.calls.length).toBeLessThanOrEqual(q.length);
      expect(mounted.core.acItems().some(item => item.label === `${trigger === "#" ? "#" : ""}Topic 0`)).toBe(true);
      expect(mounted.core.acItems().some(item => item.insert === (trigger === "#" ? `#${q}` : `[[${q}]]`))).toBe(true);
    }
  });

  it("coalesces a busy rank to the newest query, drops stale rows and preserves the baseline", async () => {
    let reply!: (rows: PageEntry[]) => void;
    const read = vi.spyOn(backend(), "quickSwitch").mockImplementationOnce(() => new Promise(resolve => { reply = resolve; }))
      .mockImplementation(async q => result(q));
    const mounted = editor();
    mounted.input("[[t"); await flush();
    expect(read).toHaveBeenCalledOnce();
    for (const q of ["to", "top", "topi", "topic", "topic 9"]) mounted.input("[[" + q);
    await flush();
    expect(read).toHaveBeenCalledOnce();
    reply([{ name: "stale", kind: "page", path: "", date_key: null }]); await flush();
    expect(read).toHaveBeenCalledTimes(2); // <= 20k candidate visits for the burst.
    expect(read.mock.calls.map(call => call[0])).toEqual(["t", "topic 9"]);
    expect(read.mock.calls[1][2]).toMatchObject({ originalRaw: "baseline [[Keep]]" });
    expect(mounted.core.acItems().some(item => item.label === "stale")).toBe(false);
    expect(mounted.core.acItems().some(item => item.label === "Topic 9")).toBe(true);
  });

  it.each(["close", "dispose", "graph", "composition"])("retires queued work on %s", async transition => {
    let reply!: (rows: PageEntry[]) => void;
    const read = vi.spyOn(backend(), "quickSwitch").mockImplementationOnce(() => new Promise(resolve => { reply = resolve; }));
    const mounted = editor(); mounted.input("[[t"); await flush();
    expect(read).toHaveBeenCalledOnce();
    mounted.input("[[to");
    if (transition === "close") mounted.core.closeAc();
    else if (transition === "composition") mounted.core.pause();
    else if (transition === "dispose") { mounted.retire(); disposals.pop()!(); }
    else invalidateBinding();
    reply([{ name: "stale", kind: "page", path: "", date_key: null }]); await flush();
    expect(read).toHaveBeenCalledOnce();
    expect(mounted.core.acItems().some(item => item.label === "stale")).toBe(false);
  });

  it("a failed read does not strand the next keystroke", async () => {
    const read = vi.spyOn(backend(), "quickSwitch").mockRejectedValueOnce(new Error("temporarily busy"))
      .mockImplementation(async q => result(q));
    const mounted = editor(); mounted.input("[[t"); await flush();
    expect(read).toHaveBeenCalledOnce();
    mounted.input("[[to"); await flush();
    expect(read).toHaveBeenCalledTimes(2);
    expect(mounted.core.acItems().some(item => item.label === "Topic 0")).toBe(true);
  });
});

function execution(query: string): QueryExecution {
  return { hits: result(query).map(page => ({ entity: "page", page, evidence: [], score: 1000,
    display_text: page.name, match_class: "prefix" })),
    diagnostics: [], explanation: { branches: [] }, has_more: { pages: false, blocks: false }, cancelled: false };
}
function switcher(pagesOnly = true) {
  openSwitcher(pagesOnly ? { mode: "pages" } : undefined);
  const root = document.createElement("div"); document.body.append(root);
  disposals.push(render(() => <QuickSwitcher />, root));
  const input = root.querySelector<HTMLInputElement>(".switcher-input")!;
  return { root, input: (q: string) => { input.value = q; input.dispatchEvent(new InputEvent("input", { bubbles: true })); } };
}

it("GH #899: quick-switcher page rows update without advancing the clock", async () => {
  const read = vi.spyOn(backend(), "runGraphSearch").mockImplementation(async q => execution(q));
  const mounted = switcher(); await flush();
  for (const [index, q] of ["t", "to", "topic 1", "topic 10"].entries()) {
    mounted.input(q); await flush();
    expect(read.mock.calls.length).toBeLessThanOrEqual(index + 1);
    expect(mounted.root.querySelector(".switcher-row .switcher-name")?.textContent).toBe(result(q)[0].name);
  }
});

it("GH #899: quick-switcher coalesces busy reads and publishes only the current query", async () => {
  let reply!: (rows: QueryExecution) => void;
  const read = vi.spyOn(backend(), "runGraphSearch").mockImplementationOnce(() => new Promise(resolve => { reply = resolve; }))
    .mockImplementation(async q => execution(q));
  const mounted = switcher(); await flush();
  mounted.input("t"); await flush(); expect(read).toHaveBeenCalledOnce();
  for (const q of ["to", "top", "topi", "topic 9"]) mounted.input(q);
  await flush(); expect(read).toHaveBeenCalledOnce();
  reply(execution("topic 1")); await flush();
  expect(read.mock.calls.map(call => call[0])).toEqual(["t", "topic 9"]);
  expect(mounted.root.textContent).toContain("Topic 9");
  expect(mounted.root.textContent).not.toContain("Topic 1");
});

it("GH #899: Ctrl+K paints the page pool before its block scan finishes", async () => {
  let finishBlocks!: (rows: QueryExecution) => void;
  const read = vi.spyOn(backend(), "runGraphSearch").mockImplementation(async (q, pages) => pages
    ? execution(q) : new Promise(resolve => { finishBlocks = resolve; }));
  const mounted = switcher(false); await flush();
  mounted.input("topic 9"); await flush();
  expect(mounted.root.textContent).toContain("Topic 9");
  expect(read.mock.calls.map(call => call.slice(0, 3))).toEqual([["topic 9", 100, 0]]);
  await vi.advanceTimersByTimeAsync(110); await flush();
  expect(read.mock.calls.map(call => call.slice(0, 3))).toEqual([["topic 9", 100, 0], ["topic 9", 0, 100]]);
  expect(mounted.root.textContent).toContain("Topic 9");
  finishBlocks({ ...execution(""), hits: [] }); await flush();
  expect(mounted.root.textContent).toContain("Topic 9");
});
