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
import { STALE_VERSION } from "../document/host/protocol";
import { bindTestHost } from "../document/host/wiring.test.support";
import { bindFileHost } from "./fileHost.test.support";

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
      createPage: async () => { throw new CreatePageRefusal("page-dirty"); },
      runGraphSearch: async () => ({ hits: [], diagnostics: [], explanation: { branches: [{ description: "ok", children: [] }] }, cancelled: false }),
    };
    const result = await materializeQueryWorkspace(input, deps);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.kind).toBe("error");
  });

  it("does not show a create error when the graph switches during a QuickSwitcher save", async () => {
    setToasts([]);
    await bindTestHost();
    vi.spyOn(backend(), "runGraphSearch").mockResolvedValue({ hits: [], diagnostics: [], explanation: { branches: [] }, cancelled: false });
    vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "absent", id: "pages/Switching.md" });
    // The old graph's host answers the held create only after the switch: its
    // session is gone, so the command is not admitted.
    let finish!: () => void;
    const save = vi.spyOn(backend(), "pageSubmit").mockImplementation(() =>
      new Promise((resolve) => { finish = () => resolve({ reason: "not-admitted" }); }));
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
    finish();
    await vi.waitFor(() => expect(toasts().some((toast) => toast.message.includes("graph changed"))).toBe(true));
    expect(toasts().filter((toast) => toast.kind === "error")).toEqual([]);
    dispose();
  });

  it("QuickSwitcher creates an empty page file; the next loaded edit saves on its created version", async () => {
    resetStore();
    const path = "pages/Created through switcher.md";
    const fh = await bindFileHost((_key, n) => (n === 1 ? "created-rev" : "edited-rev"));
    vi.spyOn(backend(), "runGraphSearch").mockResolvedValue({ hits: [], diagnostics: [], explanation: { branches: [] }, cancelled: false });
    vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "absent", id: path });
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
    await vi.waitFor(() => expect(fh.files.has(path)).toBe(true));
    expect(fh.files.get(path)!.dto.blocks.map((b) => b.raw)).toEqual([""]);
    // A create: the page was opened on no file and the text is sent as a creation.
    const [, , key, , , resolve, kinds] = fh.submit.mock.calls[0];
    expect([key, resolve, kinds]).toEqual([path, null, ["create-page"]]);
    const created = fh.versions.get(path)!;
    const loaded: PageRead = { ...fh.files.get(path)!.dto, id: path, rev: fh.files.get(path)!.rev };
    loadSingle(loaded);
    setRaw(pageByName(loaded.name)!.roots[0], "first content");
    expect(await flushPage(loaded.name)).toBe(true);
    // Authored on the created file's version: not stale, so never a conflict.
    expect(fh.submit.mock.calls.at(-1)?.[4]).toBe(created);
    expect(fh.files.get(path)!.dto.blocks.map((b) => b.raw)).toEqual(["first content"]);
    dispose();
  });

  it("QueryWorkspace materializes one query block; a later loaded edit uses the materialized revision", async () => {
    resetStore();
    const path = "pages/Saved query.md";
    const fh = await bindFileHost(() => "query-edited-rev");
    const createPage = vi.fn(async (page: PageDto) => {
      fh.files.set(path, { dto: structuredClone(page), rev: "query-created-rev" });
      return "query-created-rev";
    });
    const result = await materializeQueryWorkspace({
      title: "Saved query", sourceKind: "dsl", source: "(todo TODO)", presentation: "list", routeId: "query-pin",
    }, {
      resolvePage: async () => ({ kind: "absent", id: path }),
      createPage,
      runGraphSearch: async () => ({ hits: [], diagnostics: [], explanation: { branches: [{ description: "ok", children: [] }] }, cancelled: false }),
    });
    expect(result.ok).toBe(true);
    expect(fh.files.get(path)!.dto.blocks.map((b) => b.raw)).toEqual(["{{query (todo TODO)}}"]);
    const loaded: PageRead = { ...fh.files.get(path)!.dto, id: path, rev: fh.files.get(path)!.rev };
    loadSingle(loaded);
    setRaw(pageByName(loaded.name)!.roots[0], "{{query (todo NOW)}}");
    // The edit opens the materialized file; the host mails the version it is at.
    await vi.waitFor(() => expect(fh.versions.has(path)).toBe(true));
    const opened = fh.versions.get(path)!;
    expect(await flushPage(loaded.name)).toBe(true);
    // Authored on the materialized file's revision, so the Open granted its
    // version: an ordinary save, not a stale (conflicting) one.
    const last = fh.submit.mock.calls.at(-1)!;
    expect(last[4]).not.toBe(STALE_VERSION);
    expect(last[4]).toBe(opened);
    expect(last[3].blocks.map((b) => b.raw)).toEqual(["{{query (todo NOW)}}"]);
    expect(fh.files.get(path)!.rev).toBe("query-edited-rev");
  });
});
