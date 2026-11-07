// GH #911 review: a reload that keeps unchanged blocks must still retire the deferred
// query controls of a block whose text changed, so they cannot write the old reading back.
import { For } from "solid-js";
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { blockProperty, pageByName, resetStore } from "../document";
import { doc } from "../document/model";
import { installPageContent, loadSingle } from "../document/workingSet";
import { initParser } from "../render/parse";
import { blk, mount, page } from "../tests/blockEditorTestkit";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { openCalmQuery } from "../tests/calmQueryActions";
import { BlockList } from "./BlockList";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { baseRevFor } from "../document/host/wiring";
import type { Filter } from "../editor/queryIr";

const todoFilter: Filter = { kind: "leaf", leaf: { kind: "attr", attr: "task", op: "in", value: { kind: "list", items: [{ kind: "text", text: "TODO" }] } } };

let dispose: (() => void) | undefined;
beforeAll(initParser);
afterEach(() => {
  dispose?.(); dispose = undefined;
  vi.restoreAllMocks(); resetSharedQueryResultsForTests(); resetStore();
  document.body.replaceChildren();
});

it("a pending query reparse cannot let retained controls overwrite the external filter", async () => {
  const reading = { query: { anchor: "block" as const, filter: todoFilter,
    source: { kind: "og" as const, original: "(task TODO)", og_options: "" } }, view: { sample: 7 } };
  const parse = vi.spyOn(backend(), "parseQuery").mockImplementation(async text => {
    if (text === "(task DONE)") return new Promise(() => {});
    return reading;
  });
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([]));
  vi.spyOn(backend(), "printQuery").mockImplementation(async query => "original" in query.source ? query.source.original : "");
  vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
  loadSingle(dto(7));
  const mounted = mount(() => <For each={pageByName("Review") ? [pageByName("Review")!] : []}>{p => <div class="page-blocks"><BlockList ids={p.roots} /></div>}</For>);
  dispose = mounted.dispose;
  const { root } = mounted;
  await openCalmQuery(root, "View");
  const input = root.querySelector<HTMLInputElement>('[aria-label="Sample size"]')!;
  expect(input.value).toBe("7");
  const next = dto(7);
  next.blocks[0].raw = next.blocks[0].raw.replace("TODO", "DONE");
  next.rev = "r-external";
  installPageContent("Review", next);
  await vi.waitFor(() => expect(parse.mock.calls.some(([text]) => text === "(task DONE)")).toBe(true));
  expect(doc.byId.q.raw).toContain("(task DONE)");
  const current = root.querySelector<HTMLInputElement>('[aria-label="Sample size"]');
  if (current === input) {
    input.value = "11";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  }
  await new Promise(resolve => setTimeout(resolve, 20));
  expect(baseRevFor("Review")).toBe("r-external");
  expect(doc.byId.q.raw).toContain("(task DONE)");
});

const dto = (sample: number) => {
  const raw = `{{query (task TODO)}}\ntine.sample:: ${sample}`;
  return { ...page("Review", [{ ...blk("q", raw), properties: [["tine.sample", String(sample)]] as [string, string][] }]), id: "pages/Review.md", rev: `r${sample}` };
};

it("an untouched query sample field cannot revert an external sample edit on blur", async () => {
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (_text, _dialect, properties) => ({
    query: { anchor: "block", filter: todoFilter, source: { kind: "og", original: "(task TODO)", og_options: "" } },
    view: { sample: Number(properties?.find(([key]) => key === "tine.sample")?.[1] ?? 0) },
  }));
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([]));
  vi.spyOn(backend(), "printQuery").mockResolvedValue("(task TODO)");
  vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
  loadSingle(dto(7));
  const mounted = mount(() => <For each={pageByName("Review") ? [pageByName("Review")!] : []}>{p => <div class="page-blocks"><BlockList ids={p.roots} /></div>}</For>);
  dispose = mounted.dispose;
  const { root } = mounted;
  await vi.waitFor(() => expect(root.querySelector(".query-block")).not.toBeNull());
  await openCalmQuery(root, "View");
  const input = root.querySelector<HTMLInputElement>('[aria-label="Sample size"]')!;
  expect(input).not.toBeNull(); expect(input.value).toBe("7");
  input.focus();
  installPageContent("Review", dto(19));
  await vi.waitFor(() => expect(blockProperty("q", "tine.sample")).toBe("19"));
  await new Promise(resolve => setTimeout(resolve, 20));
  const current = root.querySelector<HTMLInputElement>('[aria-label="Sample size"]');
  if (current === input) input.dispatchEvent(new FocusEvent("blur"));
  await new Promise(resolve => setTimeout(resolve, 20));
  expect(baseRevFor("Review")).toBe("r19");
  expect(blockProperty("q", "tine.sample")).toBe("19");
});

it("an edited query sample draft of the older value is refused, not written over the external edit", async () => {
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (_text, _dialect, properties) => ({
    query: { anchor: "block", filter: todoFilter, source: { kind: "og", original: "(task TODO)", og_options: "" } },
    view: { sample: Number(properties?.find(([key]) => key === "tine.sample")?.[1] ?? 0) },
  }));
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([]));
  vi.spyOn(backend(), "printQuery").mockResolvedValue("(task TODO)");
  vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
  loadSingle(dto(7));
  const mounted = mount(() => <For each={pageByName("Review") ? [pageByName("Review")!] : []}>{p => <div class="page-blocks"><BlockList ids={p.roots} /></div>}</For>);
  dispose = mounted.dispose;
  const { root } = mounted;
  await vi.waitFor(() => expect(root.querySelector(".query-block")).not.toBeNull());
  await openCalmQuery(root, "View");
  const input = root.querySelector<HTMLInputElement>('[aria-label="Sample size"]')!;
  expect(input).not.toBeNull(); expect(input.value).toBe("7");
  input.focus();
  input.value = "11";
  input.dispatchEvent(new InputEvent("input", { bubbles: true }));
  installPageContent("Review", dto(19));
  await vi.waitFor(() => expect(blockProperty("q", "tine.sample")).toBe("19"));
  await new Promise(resolve => setTimeout(resolve, 20));
  const current = root.querySelector<HTMLInputElement>('[aria-label="Sample size"]');
  if (current === input) input.dispatchEvent(new FocusEvent("blur"));
  await new Promise(resolve => setTimeout(resolve, 20));
  expect(baseRevFor("Review")).toBe("r19");
  expect(blockProperty("q", "tine.sample")).toBe("19");
  expect(root.querySelector<HTMLInputElement>('[aria-label="Sample size"]')?.value).toBe("19");
});
