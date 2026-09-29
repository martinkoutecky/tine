// og-G2 row #52 (master cdd0eda4b317, Harvest D): ported outcome tests.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { QuickSwitcher } from "./QuickSwitcher";
import { closeSwitcher, openSwitcher, openSettings, closeSettings } from "../ui";
import { setToasts, toasts } from "../toasts";
import { route } from "../router";
import { backend } from "../backend";
import { layoutPaneIds, paneRouter, resetPaneLayoutToSingle, splitPane } from "../panes";
import { resetStore, pageByName } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { invalidateBinding } from "../binding";
import { setGraphMeta } from "../graphSession";
import { initParser } from "../render/parse";
import { startEditing } from "../editorController";
import { Block } from "./Block";
import { Settings } from "./Settings";
import type { BlockDto, PageDto } from "../types";

beforeAll(async () => { await initParser(); });

afterEach(() => {
  closeSwitcher();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
  resetStore();
  setToasts([]);
  setGraphMeta(null);
  document.body.innerHTML = "";
});

const tick = () => new Promise<void>((r) => setTimeout(r, 0));
async function settle() { for (let i = 0; i < 6; i++) await tick(); }

async function typeCreate(root: HTMLElement, name: string) {
  openSwitcher();
  const input = root.querySelector<HTMLInputElement>(".switcher-input")!;
  input.value = name;
  input.dispatchEvent(new InputEvent("input", { bubbles: true }));
  await vi.waitFor(() => expect([...root.querySelectorAll<HTMLElement>('.switcher-row[role="option"]')]
    .some((row) => row.textContent?.includes(`Create page: ${name}`))).toBe(true));
  return input;
}

function switchGraph(generation: { value: number }) {
  generation.value++;
  invalidateBinding();
  setGraphMeta({ root: "/graphs/B" } as never);
}

describe("O1 QuickSwitcher create after a graph switch (master QuickSwitcher.test.tsx)", () => {
  it("main path: does not navigate and says the graph changed", async () => {
    setGraphMeta({ root: "/graphs/A" } as never);
    const gen = { value: 1 };
    vi.spyOn(backend(), "graphBindingGeneration").mockImplementation(() => gen.value);
    let finish!: (v: { ok: string[] }) => void;
    const save = vi.spyOn(backend(), "savePages").mockImplementation(() => new Promise((r) => { finish = r as never; }));
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    try {
      await typeCreate(root, "Wrong graph page");
      const create = [...root.querySelectorAll<HTMLElement>('.switcher-row[role="option"]')]
        .find((row) => row.textContent?.includes("Create page: Wrong graph page"))!;
      create.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      await vi.waitFor(() => expect(save).toHaveBeenCalledOnce());
      switchGraph(gen);
      finish({ ok: ["created-rev"] });
      await settle();
      expect(route()).toEqual({ kind: "journals" });
      expect(toasts().some((t) => t.message.includes("graph changed"))).toBe(true);
    } finally { dispose(); }
  });

  it("Alt+Enter (other pane) path: does not open the page after the graph changed", async () => {
    setGraphMeta({ root: "/graphs/A" } as never);
    const gen = { value: 1 };
    vi.spyOn(backend(), "graphBindingGeneration").mockImplementation(() => gen.value);
    let finish!: (v: { ok: string[] }) => void;
    const save = vi.spyOn(backend(), "savePages").mockImplementation(() => new Promise((r) => { finish = r as never; }));
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    try {
      const input = await typeCreate(root, "Other pane page");
      // select the create row
      for (let i = 0; i < 20; i++) {
        const sel = root.querySelector<HTMLElement>('.switcher-row[aria-selected="true"]');
        if (sel?.textContent?.includes("Create page: Other pane page")) break;
        input.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      }
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", altKey: true, bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(save).toHaveBeenCalledOnce());
      const panesBefore = layoutPaneIds().length;
      switchGraph(gen);
      finish({ ok: ["created-rev"] });
      await settle();
      expect(layoutPaneIds().length).toBe(panesBefore);
    } finally { dispose(); }
  });
});

describe("O1 QuickSwitcher create into a new (embryo) pane after a graph switch", () => {
  it("does not open the page in the embryo pane after the graph changed", async () => {
    setGraphMeta({ root: "/graphs/A" } as never);
    const gen = { value: 1 };
    vi.spyOn(backend(), "graphBindingGeneration").mockImplementation(() => gen.value);
    let finish!: (v: { ok: string[] }) => void;
    const save = vi.spyOn(backend(), "savePages").mockImplementation(() => new Promise((r) => { finish = r as never; }));
    const other = splitPane("main", "row")!;
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    try {
      openSwitcher({ mode: "embryo", paneId: other });
      const input = root.querySelector<HTMLInputElement>(".switcher-input")!;
      input.value = "Embryo page";
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
      await vi.waitFor(() => expect([...root.querySelectorAll<HTMLElement>('.switcher-row[role="option"]')]
        .some((row) => row.textContent?.includes("Create page: Embryo page"))).toBe(true));
      [...root.querySelectorAll<HTMLElement>('.switcher-row[role="option"]')]
        .find((row) => row.textContent?.includes("Create page: Embryo page"))!
        .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      await vi.waitFor(() => expect(save).toHaveBeenCalledOnce());
      switchGraph(gen);
      finish({ ok: ["created-rev"] });
      await settle();
      expect(paneRouter(other).route()).not.toMatchObject({ kind: "page", name: "Embryo page" });
      expect(toasts().some((t) => t.message.includes("graph changed"))).toBe(true);
    } finally { dispose(); }
  });
});

describe("O3 asset landing after a graph switch (master Block.assetPaste.test.tsx)", () => {
  it("does not land a durable asset in an editor whose graph binding changed", async () => {
    setGraphMeta({ root: "/graphs/A" } as never);
    const gen = { value: 1 };
    vi.spyOn(backend(), "graphBindingGeneration").mockImplementation(() => gen.value);
    const blk = (id: string, raw: string): BlockDto => ({ id, raw, collapsed: false, children: [] });
    loadSingle({ name: "Assets", kind: "page", title: "Assets", pre_block: null, blocks: [blk("asset-stale", "")] } as PageDto);
    const id = pageByName("Assets")!.roots[0];
    startEditing(id, 0);
    vi.stubGlobal("URL", { ...URL, createObjectURL: vi.fn(() => "blob:asset"), revokeObjectURL: vi.fn() });
    let finish!: (name: string) => void;
    vi.spyOn(backend(), "saveAsset").mockImplementation(() => new Promise<string>((r) => { finish = r; }));
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render((): JSX.Element => <For each={pageByName("Assets")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>, root);
    try {
      const file = new File([new Uint8Array([1])], "paste.png", { type: "image/png" });
      const ev = new Event("paste", { bubbles: true, cancelable: true });
      Object.defineProperty(ev, "clipboardData", { value: { getData: () => "", items: [{ kind: "file", type: "image/png", getAsFile: () => file }], types: ["Files"] } });
      root.querySelector("textarea")!.dispatchEvent(ev);
      await settle();
      expect(backend().saveAsset).toHaveBeenCalledOnce();
      gen.value++;
      invalidateBinding();
      setGraphMeta({ root: "/graphs/B" } as never);
      finish("durable.png");
      await settle();
      expect(doc.byId[id].raw).toBe("");
      expect(toasts().some((t) => t.message.includes("was not inserted"))).toBe(true);
    } finally { dispose(); }
  });
});

describe("O6 Backups restore owns busy while confirmation is open (master Settings.backups.test.tsx)", () => {
  it("disables Restore while the confirmation is pending", async () => {
    setGraphMeta({ root: "/graphs/A" } as never);
    vi.spyOn(backend(), "getBackupKeep").mockResolvedValue(12);
    vi.spyOn(backend(), "listBackups").mockResolvedValue([{ stamp: "2026-07-22_12-00-00", files: 1 }] as never);
    let answer!: (c: boolean) => void;
    const confirm = vi.spyOn(backend(), "confirm").mockImplementation(() => new Promise<boolean>((r) => { answer = r; }));
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <Settings />, root);
    try {
      openSettings("backups");
      await settle();
      const restore = [...root.querySelectorAll("button")].find((b) => b.textContent?.trim() === "Restore") as HTMLButtonElement;
      expect(restore).toBeDefined();
      restore.click();
      await settle();
      expect(confirm).toHaveBeenCalledOnce();
      expect(restore.disabled).toBe(true);
      answer(false);
      await settle();
      expect(restore.disabled).toBe(false);
    } finally { closeSettings(); dispose(); }
  });
});
