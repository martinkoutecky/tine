import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { CreatePageRefusal, flushPage, pageByName, resetStore, setRaw } from "../document";
import { loadSingle } from "../document/workingSet";
import { closeSwitcher, openSwitcher } from "../ui";
import type { PageDto, PageRead } from "../types";
import { QuickSwitcher } from "./QuickSwitcher";
import { materializeQueryWorkspace } from "./QueryWorkspace";
import { setToasts, toasts } from "../toasts";

afterEach(() => {
  closeSwitcher();
  resetStore();
  setToasts([]);
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("creators that save outside the document engine", () => {
  it("does not call a local workspace refusal a disk conflict", async () => {
    const input = { title: "Saved query", sourceKind: "dsl" as const, source: "(todo TODO)", presentation: "list" as const, routeId: "query-pin" };
    const deps = {
      resolvePage: async () => ({ kind: "absent" as const, id: "pages/Saved query.md" }),
      savePages: async () => { throw new CreatePageRefusal("page-dirty"); },
      runGraphSearch: async () => ({ hits: [], diagnostics: [], explanation: { branches: [{ description: "ok", children: [] }] }, cancelled: false }),
    };
    const result = await materializeQueryWorkspace(input, deps);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.kind).toBe("error");
  });

  it("does not show a create error when the graph switches during a QuickSwitcher save", async () => {
    setToasts([]);
    vi.spyOn(backend(), "runGraphSearch").mockResolvedValue({ hits: [], diagnostics: [], explanation: { branches: [] }, cancelled: false });
    vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "absent", id: "pages/Switching.md" });
    let finish!: (result: { ok: string[] }) => void;
    const save = vi.spyOn(backend(), "savePages").mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    openSwitcher();
    const input = root.querySelector<HTMLInputElement>(".switcher-input")!;
    input.value = "Switching";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await vi.waitFor(() => expect(root.textContent).toContain("Create page: Switching"));
    const create = [...root.querySelectorAll<HTMLElement>('.switcher-row[role="option"]')]
      .find((row) => row.textContent?.includes("Create page:"))!;
    create.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    resetStore();
    finish({ ok: ["old-graph-rev"] });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(toasts().filter((toast) => toast.kind === "error")).toEqual([]);
    dispose();
  });

  it("QuickSwitcher creates an empty page file; the next loaded edit saves against its created revision", async () => {
    resetStore();
    const path = "pages/Created through switcher.md";
    const files = new Map<string, { dto: PageDto; rev: string }>();
    vi.spyOn(backend(), "runGraphSearch").mockResolvedValue({ hits: [], diagnostics: [], explanation: { branches: [] }, cancelled: false });
    vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "absent", id: path });
    const save = vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => { const { id: id, page: dto } = entries[0];
      const rev = files.has(id) ? "edited-rev" : "created-rev";
      files.set(id, { dto: structuredClone(dto), rev });
      return { ok: [rev] };
    });
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    openSwitcher();
    const input = root.querySelector<HTMLInputElement>(".switcher-input")!;
    input.value = "Created through switcher";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await vi.waitFor(() => expect(root.textContent).toContain("Create page: Created through switcher"));
    const create = [...root.querySelectorAll<HTMLElement>('.switcher-row[role="option"]')]
      .find((row) => row.textContent?.includes("Create page:"))!;
    create.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(files.has(path)).toBe(true));
    expect(files.get(path)!.dto.blocks.map((b) => b.raw)).toEqual([""]);
    expect(save.mock.calls[0][0][0].baseRev).toBeNull();
    const loaded: PageRead = { ...files.get(path)!.dto, id: path, rev: files.get(path)!.rev };
    loadSingle(loaded);
    setRaw(pageByName(loaded.name)!.roots[0], "first content");
    expect(await flushPage(loaded.name)).toBe(true);
    expect(save.mock.calls.at(-1)?.[0][0].baseRev).toBe("created-rev");
    expect(files.get(path)!.dto.blocks.map((b) => b.raw)).toEqual(["first content"]);
    dispose();
  });

  it("QueryWorkspace materializes one query block; a later loaded edit uses the materialized revision", async () => {
    resetStore();
    const path = "pages/Saved query.md";
    const files = new Map<string, { dto: PageDto; rev: string }>();
    const save = vi.fn(async (entries: import("../backend").SavePageEntry[]) => {
      const { id, page: dto } = entries[0];
      files.set(id, { dto: structuredClone(dto), rev: "query-created-rev" });
      return { ok: ["query-created-rev"] };
    });
    const result = await materializeQueryWorkspace({
      title: "Saved query", sourceKind: "dsl", source: "(todo TODO)", presentation: "list", routeId: "query-pin",
    }, {
      resolvePage: async () => ({ kind: "absent", id: path }),
      savePages: save,
      runGraphSearch: async () => ({ hits: [], diagnostics: [], explanation: { branches: [{ description: "ok", children: [] }] }, cancelled: false }),
    });
    expect(result.ok).toBe(true);
    expect(files.get(path)!.dto.blocks.map((b) => b.raw)).toEqual(["{{query (todo TODO)}}"]);
    const loaded: PageRead = { ...files.get(path)!.dto, id: path, rev: files.get(path)!.rev };
    loadSingle(loaded);
    const ordinarySave = vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["query-edited-rev"] });
    setRaw(pageByName(loaded.name)!.roots[0], "{{query (todo NOW)}}");
    expect(await flushPage(loaded.name)).toBe(true);
    expect(ordinarySave.mock.calls.at(-1)?.[0][0].baseRev).toBe("query-created-rev");
    expect(ordinarySave.mock.calls.at(-1)?.[0][0].page.blocks.map((b) => b.raw)).toEqual(["{{query (todo NOW)}}"]);
  });
});
