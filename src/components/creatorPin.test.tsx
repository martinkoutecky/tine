import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { flushPage, loadSingle, pageByName, resetStore, setRaw } from "../store";
import { closeSwitcher, openSwitcher } from "../ui";
import type { PageDto, PageRead } from "../types";
import { QuickSwitcher } from "./QuickSwitcher";
import { materializeQueryWorkspace } from "./QueryWorkspace";

afterEach(() => {
  closeSwitcher();
  resetStore();
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("creators that save outside the document engine", () => {
  it("QuickSwitcher creates an empty page file; the next loaded edit saves against its created revision", async () => {
    resetStore();
    const path = "pages/Created through switcher.md";
    const files = new Map<string, { dto: PageDto; rev: string }>();
    vi.spyOn(backend(), "runGraphSearch").mockResolvedValue({ hits: [], diagnostics: [], explanation: { branches: [] }, cancelled: false });
    vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "absent", id: path });
    const save = vi.spyOn(backend(), "savePage").mockImplementation(async (id, dto) => {
      const rev = files.has(id) ? "edited-rev" : "created-rev";
      files.set(id, { dto: structuredClone(dto), rev });
      return rev;
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
    expect(save.mock.calls[0][2]).toBeNull();
    const loaded: PageRead = { ...files.get(path)!.dto, id: path, rev: files.get(path)!.rev };
    loadSingle(loaded);
    setRaw(pageByName(loaded.name)!.roots[0], "first content");
    expect(await flushPage(loaded.name)).toBe(true);
    expect(save.mock.calls.at(-1)?.[2]).toBe("created-rev");
    expect(files.get(path)!.dto.blocks.map((b) => b.raw)).toEqual(["first content"]);
    dispose();
  });

  it("QueryWorkspace materializes one query block; a later loaded edit uses the materialized revision", async () => {
    resetStore();
    const path = "pages/Saved query.md";
    const files = new Map<string, { dto: PageDto; rev: string }>();
    const save = vi.fn(async (id: string, dto: PageDto) => {
      files.set(id, { dto: structuredClone(dto), rev: "query-created-rev" });
      return "query-created-rev";
    });
    const result = await materializeQueryWorkspace({
      title: "Saved query", sourceKind: "dsl", source: "(todo TODO)", presentation: "list", routeId: "query-pin",
    }, {
      resolvePage: async () => ({ kind: "absent", id: path }),
      savePage: save,
      runGraphSearch: async () => ({ hits: [], diagnostics: [], explanation: { branches: [{ description: "ok", children: [] }] }, cancelled: false }),
    });
    expect(result.ok).toBe(true);
    expect(files.get(path)!.dto.blocks.map((b) => b.raw)).toEqual(["{{query (todo TODO)}}"]);
    const loaded: PageRead = { ...files.get(path)!.dto, id: path, rev: files.get(path)!.rev };
    loadSingle(loaded);
    const ordinarySave = vi.spyOn(backend(), "savePage").mockResolvedValue("query-edited-rev");
    setRaw(pageByName(loaded.name)!.roots[0], "{{query (todo NOW)}}");
    expect(await flushPage(loaded.name)).toBe(true);
    expect(ordinarySave.mock.calls.at(-1)?.[2]).toBe("query-created-rev");
    expect(ordinarySave.mock.calls.at(-1)?.[1].blocks.map((b) => b.raw)).toEqual(["{{query (todo NOW)}}"]);
  });
});
