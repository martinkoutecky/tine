import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { clearClipboardPayload } from "../clipboard";
import { resetStore } from "../document";
import { endEdit } from "../editorController";
import { installKeybindings } from "../keybindings";
import { resetNearObserverForTests } from "../lazyObserve";
import { resetReferenceSectionState } from "../referenceSectionState";
import { initParser } from "../render/parse";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { maybeEditorIn } from "../tests/editorDriver";
import type { BlockDto, PageRead, RefGroup } from "../types";
import { closeExportModal } from "../ui";
import { ExportModal } from "./ExportModal";
import { PageView } from "./Page";

const block = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({
  id, raw, children, collapsed: false,
});
const page = (name: string, blocks: BlockDto[]): PageRead => ({
  name, title: name, kind: "page", format: "md", id: `pages/${name}.md`, rev: "original", pre_block: null, blocks,
});
const hit = block("alpha-hit", "Alpha reference [[Target]]", [block("alpha-child", "Alpha child")]);
const nestedHit = block("alpha-nested", "Other Alpha reference [[Target]]", [block("alpha-nested-child", "Other Alpha child")]);
const coldHit = block("beta-hit", "Beta reference [[Target]]", [
  block("beta-child", "Beta child", [block("beta-grandchild", "Beta grandchild")]),
]);
const excludedHit = block("gamma-hit", "Gamma reference [[Target]]");
const target = page("Target", [block("target-root", "Target page body")]);
const sources = [
  page("Alpha", [hit, block("alpha-parent", "Unselected parent", [nestedHit])]),
  page("Beta", [coldHit, block("beta-unrelated", "Unrelated Beta block")]),
  page("Gamma", [excludedHit]),
];
const shallow = (value: BlockDto) => ({ ...value, children: [] });
const groups: RefGroup[] = sources.map((source, index) => ({
  page: source.name, kind: source.kind, path: source.id,
  blocks: [shallow([hit, coldHit, excludedHit][index]), ...(index === 0 ? [shallow(nestedHit)] : [])],
}));

const disposers: (() => void)[] = [];
beforeAll(initParser);
afterEach(() => {
  endEdit("page-navigation");
  for (const dispose of disposers.splice(0).reverse()) dispose();
  closeExportModal();
  clearClipboardPayload();
  resetStore();
  resetTabsToJournals();
  resetNearObserverForTests();
  resetReferenceSectionState();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  localStorage.clear();
  document.body.replaceChildren();
});

async function mountTarget(sourcePages = sources, referenceGroups = groups) {
  vi.stubGlobal("IntersectionObserver", class {
    constructor(private callback: IntersectionObserverCallback) {}
    observe(target: Element) {
      const group = target.closest(".reference-group");
      const isIntersecting = !group || group.querySelector(".reference-page")?.textContent === "Alpha";
      queueMicrotask(() => this.callback([{ target, isIntersecting } as IntersectionObserverEntry], this as unknown as IntersectionObserver));
    }
    unobserve() {}
    disconnect() {}
  });
  vi.spyOn(backend(), "getPage").mockImplementation(async name => name === target.name ? target : null);
  const readByPath = vi.spyOn(backend(), "getPageByPath").mockImplementation(async path => sourcePages.find(source => source.id === path) ?? null);
  const backlinks = vi.spyOn(backend(), "getBacklinks").mockResolvedValue(referenceGroups);
  const writeText = vi.spyOn(backend(), "writeText").mockResolvedValue(undefined);
  const writeRich = vi.spyOn(backend(), "writeRich").mockResolvedValue(undefined);
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Target", pageKind: "page" });
  const root = document.createElement("div");
  document.body.append(root);
  disposers.push(installKeybindings(), render(() => <><PageView /><ExportModal /></>, root));
  const liveRow = (id: string) => root.querySelector<HTMLElement>(`.ls-block:not(.ref-block)[data-block-id="${id}"] > .block-main`);
  await vi.waitFor(() => expect(readByPath).toHaveBeenCalledWith("pages/Alpha.md"));
  await vi.waitFor(() => expect(liveRow(hit.id)).not.toBeNull());
  return { root, liveRow, readByPath, backlinks, writeText, writeRich };
}

function chooseReferences(root: HTMLElement, keep: (label: string) => boolean) {
  root.querySelector<HTMLButtonElement>('button[aria-label="Copy / export linked references"]')!.click();
  const chooser = root.querySelector('[role="dialog"][aria-label="Copy / export Linked References"]')!;
  for (const label of chooser.querySelectorAll("label")) {
    if (!keep(label.textContent ?? "")) label.querySelector("input")!.click();
  }
  return chooser.querySelector<HTMLButtonElement>(".export-btn-primary")!;
}

function changeExportLevel(root: HTMLElement, value: string) {
  const level = root.querySelector<HTMLSelectElement>(".export-modal select")!;
  level.value = value;
  level.dispatchEvent(new Event("change", { bubbles: true }));
}

function press(key: string, options: KeyboardEventInit = {}) {
  document.body.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...options }));
}

async function selectAlpha(root: HTMLElement) {
  const content = root.querySelector<HTMLElement>(`[data-block-id="${hit.id}"] > .block-main .block-content`)!;
  content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  const editor = await vi.waitFor(() => {
    const editor = maybeEditorIn(root, hit.id);
    expect(editor).not.toBeNull();
    return editor!;
  });
  editor.keydown({ key: "Escape" });
  for (let i = 0; i < 4; i++) press("a", { ctrlKey: true });
  press("c", { ctrlKey: true });
}

it("copies selected cold reference trees to the requested level on a routed page", async () => {
  const { root, liveRow, readByPath, writeText } = await mountTarget();
  expect(liveRow(coldHit.id)).toBeNull();
  root.querySelector<HTMLButtonElement>('button[aria-label="Copy / export linked references"]')!.click();
  const chooser = root.querySelector('[role="dialog"][aria-label="Copy / export Linked References"]')!;
  for (const label of chooser.querySelectorAll("label")) {
    if (!label.textContent?.includes("Beta reference")) label.querySelector("input")!.click();
  }
  chooser.querySelector<HTMLButtonElement>(".export-btn-primary")!.click();
  const preview = () => root.querySelector<HTMLTextAreaElement>(".export-preview")?.value ?? "";
  await vi.waitFor(() => expect(preview()).toContain("Beta reference"));
  const level = root.querySelector<HTMLSelectElement>("label select")!;
  const setLevel = (value: string) => {
    level.value = value;
    level.dispatchEvent(new Event("change", { bubbles: true }));
  };
  setLevel("2");
  expect(preview()).toContain("Beta child");
  expect(preview()).not.toMatch(/Beta grandchild|Unrelated Beta block|Alpha reference|Gamma reference/);
  setLevel("1");
  expect(preview()).toContain("Beta reference");
  expect(preview()).not.toContain("Beta child");
  setLevel("all");
  expect(preview()).toContain("Beta grandchild");
  const format = (name: string) => [...root.querySelectorAll<HTMLButtonElement>(".export-modal button")]
    .find(button => button.textContent?.trim() === name)!.click();
  setLevel("2");
  for (const name of ["OPML", "HTML"]) {
    format(name);
    expect(preview()).toContain("Beta child");
    expect(preview()).not.toContain("Beta grandchild");
  }
  format("Text");
  setLevel("all");
  const copy = root.querySelector<HTMLButtonElement>(".export-modal .export-btn-primary")!;
  await vi.waitFor(() => expect(copy.disabled).toBe(false));
  copy.click();
  expect(writeText).toHaveBeenCalledWith(expect.stringContaining("Beta grandchild"));
  expect(readByPath.mock.calls).toEqual([["pages/Alpha.md"], ["pages/Beta.md"]]);
  expect(liveRow(coldHit.id)).toBeNull();
});

it("uses the loaded source subtree without reading the page again", async () => {
  const { root, readByPath } = await mountTarget();
  root.querySelector<HTMLButtonElement>('button[aria-label="Copy / export linked references"]')!.click();
  const chooser = root.querySelector('[role="dialog"][aria-label="Copy / export Linked References"]')!;
  for (const label of chooser.querySelectorAll("label")) {
    if (!label.textContent?.includes("Alpha reference")) label.querySelector("input")!.click();
  }
  chooser.querySelector<HTMLButtonElement>(".export-btn-primary")!.click();
  const preview = () => root.querySelector<HTMLTextAreaElement>(".export-preview")?.value ?? "";
  await vi.waitFor(() => expect(preview()).toContain("Other Alpha child"));
  expect(preview()).toContain("Alpha child");
  expect(preview()).not.toMatch(/Unselected parent|Beta reference|Gamma reference/);
  expect(readByPath.mock.calls).toEqual([["pages/Alpha.md"]]);
});

it("does not reopen export after the chooser is dismissed during a cold source read", async () => {
  const { root, readByPath } = await mountTarget();
  let resolveSource!: (source: PageRead) => void;
  const sourceRead = new Promise<PageRead>(resolve => { resolveSource = resolve; });
  readByPath.mockReturnValueOnce(sourceRead);
  root.querySelector<HTMLButtonElement>('button[aria-label="Copy / export linked references"]')!.click();
  const chooser = root.querySelector('[role="dialog"][aria-label="Copy / export Linked References"]')!;
  chooser.querySelector<HTMLButtonElement>(".export-btn-primary")!.click();
  expect(readByPath).toHaveBeenLastCalledWith("pages/Beta.md");
  for (const control of chooser.querySelectorAll<HTMLInputElement | HTMLButtonElement>("input, .ref-export-actions button")) {
    expect(control.disabled).toBe(true);
  }
  const cancel = chooser.querySelector<HTMLButtonElement>(".export-btn-secondary")!;
  expect(cancel.disabled).toBe(false);
  cancel.click();
  resolveSource(sources[1]);
  await sourceRead;
  await Promise.resolve();
  expect(root.querySelector(".export-preview")).toBeNull();
  expect(readByPath.mock.calls).toEqual([["pages/Alpha.md"], ["pages/Beta.md"]]);
});

it.each([["Alpha", "1"], ["Alpha", "2"], ["Beta", "1"], ["Beta", "2"]])(
  "promotes checked %s rows below level %s in every format", async (name, level) => {
    const sourceIndex = name === "Alpha" ? 0 : 1;
    const parent = name === "Alpha" ? hit : coldHit;
    const selected = block("checked-descendant", "Checked descendant [[Target]]", [block("descendant-leaf", "Descendant leaf")]);
    const descendant = level === "1" ? selected : block("unchecked-bridge", "Unchecked intermediate", [selected]);
    const tree = { ...parent, children: [descendant] };
    const sourcePages = sources.map((source, index) => index === sourceIndex ? { ...source, blocks: [tree] } : source);
    const referenceGroups = groups.map((group, index) => index === sourceIndex ? { ...group, blocks: [shallow(tree), shallow(selected)] } : group);
    const { root } = await mountTarget(sourcePages, referenceGroups);
    chooseReferences(root, label => label === tree.raw || label === selected.raw).click();
    const preview = await vi.waitFor(() => {
      const field = root.querySelector<HTMLTextAreaElement>(".export-preview");
      expect(field).not.toBeNull();
      return field!;
    });
    changeExportLevel(root, level);
    expect(preview.value.split("\n").some(line => line.startsWith("\t- Checked descendant"))).toBe(true);
    if (level === "2") expect(preview.value.split("\n")).toContain("\t\t- Descendant leaf");
    const format = (label: string) => [...root.querySelectorAll<HTMLButtonElement>(".export-modal button")]
      .find(button => button.textContent?.trim() === label)!.click();
    format("OPML");
    const opml = new DOMParser().parseFromString(preview.value, "application/xml");
    expect([...opml.querySelectorAll("body > outline > outline")].map(node => node.getAttribute("text")?.split(" [[")[0]))
      .toEqual([tree.raw.split(" [[")[0], "Checked descendant"]);
    format("HTML");
    const html = new DOMParser().parseFromString(preview.value, "text/html");
    expect([...html.querySelectorAll("body > ul > li > ul > li")].map(node => node.firstChild?.textContent?.split(" [[")[0]))
      .toEqual([tree.raw.split(" [[")[0], "Checked descendant"]);
    changeExportLevel(root, "all");
    expect(preview.value.match(/Checked descendant/g)).toHaveLength(1);
  },
);

it("copies selected roots from different source depths as siblings, with their children nested as in Logseq", async () => {
  const { root, writeRich } = await mountTarget();
  await selectAlpha(root);
  const [plain, html] = writeRich.mock.calls[0]!;
  expect(plain.split("\n")).toEqual([
    "- Alpha reference [[Target]]", "\t- Alpha child",
    "- Other Alpha reference [[Target]]", "\t- Other Alpha child",
  ]);
  const copied = new DOMParser().parseFromString(html, "text/html");
  const roots = [...copied.body.querySelectorAll(":scope > ul > li")];
  expect(roots.map(root => root.firstChild?.textContent)).toEqual(["Alpha reference [[Target]]", "Other Alpha reference [[Target]]"]);
  expect(roots.map(root => root.querySelector("ul li")?.textContent)).toEqual(["Alpha child", "Other Alpha child"]);
});
