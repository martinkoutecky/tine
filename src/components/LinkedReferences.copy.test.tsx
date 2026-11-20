import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { clearClipboardPayload } from "../clipboard";
import * as cut from "../cut";
import { deleteBlock, blankOutlineSelection, extendSelectionTo, node, resetStore, selectedIds, topSelected } from "../document";
import { setDoc } from "../document/model";
import { endEdit } from "../editorController";
import { setGraphMeta } from "../graphSession";
import { installKeybindings } from "../keybindings";
import { resetNearObserverForTests } from "../lazyObserve";
import { exitPaneSelect } from "../paneSelect";
import { resetReferenceSectionState } from "../referenceSectionState";
import { initParser } from "../render/parse";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { openCommentThread } from "../tests/commentEntry";
import { maybeEditorIn } from "../tests/editorDriver";
import { backendReadsQueries, blockRunResult } from "../tests/queryReadingsTestkit";
import type { BlockDto, PageRead, RefGroup } from "../types";
import { closeExportModal, requestBlockReferences, setBlockReferencesRequest } from "../ui";
import { ExportModal } from "./ExportModal";
import { LiveRefGroup } from "./LiveRefGroup";
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
  setBlockReferencesRequest(null);
  clearClipboardPayload();
  resetStore();
  exitPaneSelect();
  setGraphMeta(null);
  resetTabsToJournals();
  resetNearObserverForTests();
  resetReferenceSectionState();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  localStorage.clear();
  document.body.replaceChildren();
});

async function mountTarget(sourcePages = sources, referenceGroups = groups, visibleSources = ["Alpha"], routedPage = target) {
  const observed = new Map<Element, (() => void)[]>();
  const showSource = (name: string) => {
    for (const [element, reveal] of observed) {
      if (element.closest(".reference-group")?.querySelector(".reference-page")?.textContent === name) {
        for (const notify of reveal) notify();
      }
    }
  };
  vi.stubGlobal("IntersectionObserver", class {
    constructor(private callback: IntersectionObserverCallback) {}
    observe(target: Element) {
      const group = target.closest(".reference-group");
      const isIntersecting = !group || visibleSources.includes(group.querySelector(".reference-page")?.textContent ?? "");
      const notify = (visible: boolean) => this.callback([{ target, isIntersecting: visible } as IntersectionObserverEntry], this as unknown as IntersectionObserver);
      const callbacks = observed.get(target) ?? [];
      callbacks.push(() => notify(true));
      observed.set(target, callbacks);
      queueMicrotask(() => notify(isIntersecting));
    }
    unobserve() {}
    disconnect() {}
  });
  vi.spyOn(backend(), "getPage").mockImplementation(async name => name === routedPage.name ? routedPage : sourcePages.find(source => source.name === name) ?? null);
  const readByPath = vi.spyOn(backend(), "getPageByPath").mockImplementation(async path => path === routedPage.id ? routedPage : sourcePages.find(source => source.id === path) ?? null);
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
  return { root, liveRow, readByPath, backlinks, writeText, writeRich, showSource };
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

function press(key: string, options: KeyboardEventInit = {}, target: Element = document.body) {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...options });
  target.dispatchEvent(event);
  return event;
}

async function openBlockEditor(root: ParentNode, id: string) {
  const content = root.querySelector(`[data-block-id="${id}"] > .block-main .block-content`)!;
  content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  return vi.waitFor(() => {
    const editor = maybeEditorIn(root, id);
    expect(editor).not.toBeNull();
    return editor!;
  });
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
  const editor = await openBlockEditor(root, hit.id);
  editor.keydown({ key: "Escape" });
  for (let i = 0; i < 4; i++) press("a", { ctrlKey: true });
  press("c", { ctrlKey: true });
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

it("keeps child and grandchild levels after Ctrl+A with multiple reference groups loaded", async () => {
  const grandchild = block("alpha-grandchild", "Alpha grandchild");
  const child = block("alpha-child", "Alpha child [[Target]]", [grandchild]);
  const tree = { ...hit, children: [child] };
  const sourcePages = [{ ...sources[0], blocks: [tree, sources[0].blocks[1]] }, ...sources.slice(1)];
  const referenceGroups = [{ ...groups[0], blocks: [...groups[0].blocks, shallow(child)] }, ...groups.slice(1)];
  setGraphMeta({ root: "/graphs/A", ref_default_open_blocks_level: 5 } as never);
  const { root, liveRow, readByPath, writeRich } = await mountTarget(sourcePages, referenceGroups, ["Alpha", "Beta", "Gamma"]);
  await vi.waitFor(() => {
    for (const id of [grandchild.id, nestedHit.id, "beta-grandchild", excludedHit.id]) {
      expect(liveRow(id), id).not.toBeNull();
    }
  });
  const readsBeforeCopy = readByPath.mock.calls.length;
  const mouseEditor = await openBlockEditor(root, hit.id);
  mouseEditor.keydown({ key: "Escape" });
  expect(press("c", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(writeRich).toHaveBeenCalledOnce();
  const singleTree = writeRich.mock.calls[0];

  const editor = await openBlockEditor(root, hit.id);
  editor.keydown({ key: "a", ctrlKey: true });
  // jsdom has no native textarea select-all default action.
  if (editor.kind === "textarea") editor.selectByUser(0, editor.value().length);
  expect(editor.keydown({ key: "a", ctrlKey: true }).defaultPrevented).toBe(true);
  for (let i = 0; i < 2; i++) expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(press("c", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(writeRich).toHaveBeenCalledTimes(2);
  expect(readByPath).toHaveBeenCalledTimes(readsBeforeCopy);

  const [plain, html] = writeRich.mock.calls[1];
  expect(singleTree[0]).toBe("- Alpha reference [[Target]]\n\t- Alpha child [[Target]]\n\t\t- Alpha grandchild");
  expect(plain).toBe(`${singleTree[0]}\n- Other Alpha reference [[Target]]\n\t- Other Alpha child`);
  const copied = new DOMParser().parseFromString(html, "text/html");
  const roots = [...copied.body.querySelectorAll(":scope > ul > li")];
  const single = new DOMParser().parseFromString(singleTree[1], "text/html");
  expect(roots.map(root => root.firstChild?.textContent)).toEqual([hit.raw, nestedHit.raw]);
  for (const [label, document] of [["single tree", single], ["Ctrl+A", copied]] as const) {
    const firstChild = document.body.querySelector(":scope > ul > li > ul > li");
    expect(firstChild?.firstChild?.textContent, label).toBe(child.raw);
    expect(firstChild?.querySelector(":scope > ul > li")?.textContent, label).toBe(grandchild.raw);
  }
  expect(roots[1].querySelector(":scope > ul > li")?.textContent).toBe(nestedHit.children[0].raw);
});

it("copies only the clicked group after repeated select all", async () => {
  setGraphMeta({ root: "/graphs/A", ref_default_open_blocks_level: 5 } as never);
  const grandchild = block("alpha-grandchild", "Alpha grandchild");
  const tree = { ...hit, children: [{ ...hit.children[0], children: [grandchild] }] };
  const { root, liveRow, readByPath, writeRich } = await mountTarget(
    [{ ...sources[0], blocks: [tree, sources[0].blocks[1]] }, ...sources.slice(1)], groups, ["Alpha", "Beta"],
  );
  await vi.waitFor(() => expect(liveRow("beta-grandchild")).not.toBeNull());
  const reads = readByPath.mock.calls.length;
  const submit = vi.spyOn(backend(), "pageSubmit");
  expect(readByPath).not.toHaveBeenCalledWith("pages/Gamma.md");
  root.querySelector<HTMLElement>(".linked-references")!.click();
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(false);
  expect(selectedIds()).toEqual([]);
  const [alphaGroup, betaGroup, gammaGroup] = root.querySelectorAll<HTMLElement>(".reference-group");
  root.tabIndex = -1;
  root.focus(); // The browser gives the clicked group's pane scroller keyboard focus.
  alphaGroup.click();
  expect(press("a", { ctrlKey: true }, root).defaultPrevented).toBe(true);
  expect(press("a", { ctrlKey: true }, root).defaultPrevented).toBe(true);
  expect(press("c", { ctrlKey: true }, root).defaultPrevented).toBe(true);
  expect(writeRich).toHaveBeenCalledTimes(1);
  expectClipboard(writeRich.mock.lastCall!, [tree, nestedHit]);
  expect(selectedIds()).toEqual([]);
  expect(press("Delete").defaultPrevented).toBe(false);
  betaGroup.querySelector<HTMLButtonElement>(".reference-group-disclosure")!.click();
  expect(press("c", { ctrlKey: true }).defaultPrevented).toBe(false);
  gammaGroup.click();
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(false);
  expect(press("c", { ctrlKey: true }).defaultPrevented).toBe(false);
  expect(readByPath).toHaveBeenCalledTimes(reads);
  expect(submit).not.toHaveBeenCalled();
});

it("selects the routed page outline from blank space and preserves native select-all in fields", async () => {
  const { root, writeRich } = await mountTarget();
  const body = root.querySelector<HTMLElement>(".page-blocks")!;
  for (const field of [document.createElement("input"), document.createElement("textarea"), document.createElement("div")]) {
    if (field.tagName === "DIV") field.setAttribute("contenteditable", "true");
    root.append(field);
    field.focus();
    expect(press("a", { ctrlKey: true }, field).defaultPrevented).toBe(false);
    field.remove();
  }
  body.click();
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(press("c", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(writeRich.mock.calls[0][0]).toBe("- Target page body");
  const dialog = document.createElement("div");
  dialog.setAttribute("role", "dialog");
  root.append(dialog);
  body.click();
  expect(press("a", { ctrlKey: true }, dialog).defaultPrevented).toBe(false);
  expect(selectedIds()).toEqual([]);
});

it.each([false, true])("copies every reference root when expanded context repeats the first hit (trailing group: %s)", async (trailing) => {
  const other = block("alpha-other", "Second top-level hit [[Target]]");
  const trailingHit = block("alpha-trailing", "Trailing nested hit [[Target]]");
  const blocks = [hit, other, ...(trailing ? [block("alpha-trail-parent", "Trailing context", [trailingHit])] : [])];
  const hits = [hit, other, hit.children[0], ...(trailing ? [trailingHit] : [])];
  const { root, liveRow, writeRich } = await mountTarget(
    [{ ...sources[0], blocks }, ...sources.slice(1)],
    [{ ...groups[0], blocks: hits.map(shallow) }, ...groups.slice(1)], ["Alpha", "Beta"],
  );
  await vi.waitFor(() => expect(liveRow(coldHit.id)).not.toBeNull());
  const alpha = root.querySelector<HTMLElement>(".linked-references .reference-group")!;
  alpha.querySelector<HTMLElement>(".ref-crumb[role=button]")!.click();
  expect(alpha.querySelectorAll(`[data-block-id="${hit.id}"]`)).toHaveLength(2);
  alpha.click();
  const expected = [hit.raw, other.raw, ...(trailing ? [trailingHit.raw] : [])];
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(press("c", { ctrlKey: true }).defaultPrevented).toBe(true);
  const [plain, html] = writeRich.mock.lastCall!;
  expect(plain.split("\n").filter(line => line.startsWith("- "))).toEqual(expected.map(raw => `- ${raw}`));
  expect(plain).toContain(`\n\t- ${hit.children[0].raw}`);
  const copied = new DOMParser().parseFromString(html, "text/html");
  expect([...copied.querySelectorAll("body > ul > li")].map(li => li.firstChild?.textContent)).toEqual(expected);
});

it.each(["group to group", "Linked to Unlinked", "page to embed"])("starts in the newly clicked outline from %s", async (journey) => {
  const embedded = page("Embedded", [block("embedded-root", "Embedded outline")]);
  const embedHost = block("embed-host", "{{embed [[Embedded]]}}");
  const withEmbed = { ...target, blocks: [...target.blocks, embedHost] };
  const mention = block("gamma-mention", "Plain mention of Target");
  const gamma = { ...sources[2], blocks: [{ ...excludedHit, children: [mention] }] };
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([{ ...groups[2], blocks: [shallow(mention)] }]);
  const { root, liveRow, writeRich } = await mountTarget([...sources.slice(0, 2), gamma, embedded], groups, ["Alpha", "Beta", "Gamma"], withEmbed);
  const [alpha, beta] = root.querySelectorAll<HTMLElement>(".linked-references .reference-group");
  await vi.waitFor(() => expect(liveRow(coldHit.id)).not.toBeNull());
  let from = alpha, to = beta, first = coldHit.raw;
  let expected = [coldHit.id, coldHit.children[0].id, coldHit.children[0].children[0].id];
  if (journey === "Linked to Unlinked") {
    root.querySelector<HTMLElement>(".unlinked-references .references-header")!.click();
    const crumb = await vi.waitFor(() => {
      const crumb = root.querySelector<HTMLElement>(".unlinked-references .ref-crumb[role=button]");
      expect(crumb).not.toBeNull();
      return crumb!;
    });
    to = crumb.closest<HTMLElement>(".reference-group")!;
    crumb.click();
    first = excludedHit.raw;
    expected = [excludedHit.id, mention.id];
  } else if (journey === "page to embed") {
    from = root.querySelector<HTMLElement>(".page-blocks")!;
    to = await vi.waitFor(() => {
      const embed = root.querySelector<HTMLElement>(".embed-block");
      expect(embed?.textContent).toContain("Embedded outline");
      return embed!;
    });
    first = "Embedded outline";
    expected = [embedded.blocks[0].id];
  }
  root.tabIndex = -1;
  root.focus();
  from.click();
  expect(press("a", { ctrlKey: true }, root).defaultPrevented).toBe(true);
  expect(selectedIds().length).toBeGreaterThan(0);
  to.click();
  expect(selectedIds()).toEqual([]);
  expect(press("a", { ctrlKey: true }, root).defaultPrevented).toBe(true);
  expect(selectedIds()).toEqual(expected);
  expect(press("c", { ctrlKey: true }, root).defaultPrevented).toBe(true);
  expect(writeRich.mock.lastCall?.[0].split("\n")[0]).toBe(`- ${first}`);
});

function hitBlank(at: Element) {
  at.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0 }));
  at.dispatchEvent(new MouseEvent("pointerup", { bubbles: true, button: 0 }));
  at.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 }));
}

it.each(["Shift-click", "click elsewhere", "Escape"])("ends blank selection on %s before two Deletes", async gesture => {
  const sibling = block("untouched-sibling", "Undisplayed source sibling");
  const { root, liveRow } = await mountTarget([
    { ...sources[0], blocks: [hit, { ...sources[0].blocks[1], children: [nestedHit, sibling] }] },
    ...sources.slice(1),
  ], groups, ["Alpha", "Beta"]);
  await vi.waitFor(() => expect(liveRow(coldHit.id)).not.toBeNull());
  const submit = vi.spyOn(backend(), "pageSubmit");
  const group = liveRow(hit.id)!.closest(".reference-group")!;
  hitBlank(group);
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(selectedIds()).toContain(nestedHit.id);
  if (gesture === "Shift-click") {
    const content = liveRow(hit.id)!.querySelector(".block-content-wrapper")!;
    for (const type of ["pointerdown", "mousedown", "click"]) {
      content.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, shiftKey: true, detail: 1 }));
    }
  } else if (gesture === "click elsewhere") {
    hitBlank(root);
  } else press("Escape");
  const afterGesture = selectedIds();
  const deleted = [press("Delete"), press("Delete")];
  expect(afterGesture).toEqual([]);
  expect(selectedIds()).toEqual([]);
  expect(deleted.every(e => !e.defaultPrevented)).toBe(true);
  for (const block of [hit, hit.children[0], nestedHit, sibling, coldHit, target.blocks[0]]) {
    expect(node(block.id)?.raw).toBe(block.raw);
  }
  expect(submit).not.toHaveBeenCalled();
});

it.each([".embed-action-source", ".qc-actions summary", "inside one dialog"].flatMap(control =>
  [false, true].flatMap(selected => ["Delete", "Backspace", "Cut"].map(action => ({ control, selected, action }))),
))(
  "retires a blank page hit on focus change to $control before $action (already selected: $selected)", async ({ control, selected, action }) => {
    const embedded = page("Embedded", [block("focus-embed-root", "Embed source")]);
    const routed = { ...target, blocks: [
      ...target.blocks, block("focus-embed-host", "{{embed [[Embedded]]}}"),
      block("focus-query-host", "{{query (task TODO)}}"),
    ] };
    backendReadsQueries({ "(task TODO)": { form: "(task TODO)" } });
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([groups[1]]));
    const { root, writeRich } = await mountTarget([...sources, embedded], groups, ["Alpha", "Beta", "Embedded"], routed);
    if (control === "inside one dialog") {
      root.setAttribute("role", "dialog");
      const first = root.appendChild(document.createElement("button"));
      root.appendChild(document.createElement("button")).className = "next-dialog-control";
      first.focus();
    }
    const button = await vi.waitFor(() => {
      const element = root.querySelector<HTMLElement>(control === "inside one dialog" ? ".next-dialog-control" : control);
      expect(element).not.toBeNull();
      return element!;
    });
    const submit = vi.spyOn(backend(), "pageSubmit");
    hitBlank(root.querySelector(".page-blocks")!);
    if (selected) {
      expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
      expect(selectedIds()).toEqual(routed.blocks.map(block => block.id));
    }
    button.focus();
    expect(document.activeElement).toBe(button);
    const afterFocus = selectedIds();
    const select = press("a", { ctrlKey: true }, button);
    const remove = pressAction(action, button);
    expect(afterFocus).toEqual([]);
    expect(select.defaultPrevented).toBe(false);
    expect(remove.defaultPrevented).toBe(false);
    expect(selectedIds()).toEqual([]);
    for (const block of [...routed.blocks, embedded.blocks[0], hit, coldHit]) {
      expect(node(block.id)?.raw).toBe(block.raw);
    }
    expect(submit).not.toHaveBeenCalled();
    expect(writeRich).not.toHaveBeenCalled();
  },
);

const blankSurfaces = [
  ["Linked References", ".linked-references", coldHit.id],
  ["Unlinked References", ".unlinked-references", "unlinked-root"],
  ["embed", ".embed-block", "embedded-root"],
  ["page body", ".page-blocks", "target-root"],
  ["query results", ".query-group", coldHit.id],
  ["block references", ".block-references-inner", coldHit.id],
] as const;
const removalKeys = ["Delete", "Backspace", "Cut"];
const flatten = (blocks: BlockDto[]): BlockDto[] => blocks.flatMap(block => [block, ...flatten(block.children)]);

function pressAction(action: string, target?: Element) {
  if (action === "Copy" || action === "Cut") return press(action === "Copy" ? "c" : "x", { ctrlKey: true }, target);
  return press(action, {}, target);
}

it.each(["missing embed", "self embed", "folded embed", "collapsed reference", "cold reference", "reference excerpt"].flatMap(
  zone => removalKeys.map(action => ({ zone, action })),
))("leaves Ctrl+A and $action native on a $zone", async ({ zone, action }) => {
  const embedded = page("Embedded", [block("embedded-root", "Embedded source")]);
  const mention = block("unlinked-mention", "Plain mention of Target");
  const unlinked = page("Unlinked", [mention]);
  const routed = { ...target, blocks: [
    ...target.blocks, block("missing-embed", "{{embed [[Missing]]}}"), block("self-embed", "{{embed [[Target]]}}"),
    { ...block("folded-embed", "{{embed [[Embedded]]}}"), collapsed: true },
  ] };
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([{ page: unlinked.name, path: unlinked.id, kind: unlinked.kind, blocks: [mention] }]);
  const { root, readByPath, writeRich } = await mountTarget([...sources, embedded, unlinked], groups, ["Alpha", "Unlinked"], routed);
  if (zone === "collapsed reference") root.querySelector<HTMLButtonElement>(".reference-group-disclosure")!.click();
  if (zone === "reference excerpt") root.querySelector<HTMLButtonElement>(".unlinked-references .references-header")!.click();
  const targets: Record<string, string> = {
    "missing embed": '[data-block-id="missing-embed"] .embed-block',
    "self embed": '[data-block-id="self-embed"] .embed-block',
    "folded embed": '[data-block-id="folded-embed"] .embed-block',
    "collapsed reference": ".linked-references .reference-group .reference-group-header",
    "cold reference": ".linked-references .reference-group:last-of-type",
    "reference excerpt": '.unlinked-references [data-reference-block="unlinked-mention"]',
  };
  const at = await vi.waitFor(() => {
    const el = root.querySelector(targets[zone]);
    expect(el).toBeTruthy();
    if (zone === "reference excerpt") expect(node(mention.id)?.raw).toBe(mention.raw);
    return el!;
  });
  const from = flatten([...routed.blocks, ...sources[0].blocks, ...(zone === "reference excerpt" ? [mention] : [])]);
  const reads = readByPath.mock.calls.length;
  const cuts = vi.spyOn(cut, "cutBlocks");
  hitBlank(at);
  const select = press("a", { ctrlKey: true }).defaultPrevented;
  const roots = topSelected();
  const remove = pressAction(action).defaultPrevented;
  await cuts.mock.results.at(-1)?.value;
  expect({ select, roots, remove, source: from.map(block => node(block.id)?.raw) }).toEqual({
    select: false, roots: [], remove: false, source: from.map(block => block.raw),
  });
  expect(readByPath).toHaveBeenCalledTimes(reads);
  expect(writeRich).not.toHaveBeenCalled();
});

it.each(["Linked", "Unlinked"].flatMap(section => ["gap", "Shift-click", "Shift+Arrow"].flatMap(gesture =>
  removalKeys.map(action => ({ section, gesture, action })),
)))("keeps $section groups separate after $gesture and $action", async ({ section, gesture, action }) => {
  const unlinked = section === "Unlinked";
  const first = unlinked ? block("alpha-context", "First context", [block("alpha-mention", "A mention of Target")]) : hit;
  const second = unlinked ? block("beta-context", "Second context", [block("beta-mention", "Another mention of Target")]) : coldHit;
  const pages = [{ ...sources[0], blocks: [first] }, { ...sources[1], blocks: [second] }];
  const hits = pages.map(({ name, kind, id, blocks }) => ({ page: name, kind, path: id, blocks: (unlinked ? blocks[0].children : blocks).map(shallow) }));
  if (unlinked) vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue(hits);
  // The mount fixture uses the first Linked hit as its ready signal.
  const { root, writeRich } = await mountTarget(unlinked
    ? [{ ...pages[0], blocks: [hit, first] }, ...pages.slice(1)] : pages,
    unlinked ? [groups[0]] : hits, ["Alpha", "Beta"]);
  const panel = root.querySelector<HTMLElement>(unlinked ? ".unlinked-references" : ".linked-references")!;
  if (unlinked) panel.querySelector<HTMLButtonElement>(".references-header")!.click();
  const cards = await vi.waitFor(() => {
    const cards = [...panel.querySelectorAll<HTMLElement>(".reference-group")];
    expect(cards).toHaveLength(2);
    for (const card of cards) expect(card.querySelector(unlinked ? ".ref-crumb[role=button]" : ".ls-block:not(.ref-block)")).toBeTruthy();
    return cards;
  });
  if (unlinked) for (const card of cards) card.querySelector<HTMLElement>(".ref-crumb[role=button]")!.click();
  const chrome = cards[1].querySelector(".reference-group-header")!;
  hitBlank(chrome);
  for (let i = 0; i < 2; i++) {
    expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
    expect(topSelected()).toEqual([second.id]);
  }
  expect(pressAction("Copy").defaultPrevented).toBe(true);
  expectClipboard(writeRich.mock.lastCall!, [second]);
  expect(topSelected()).toEqual([]);
  writeRich.mockClear();
  hitBlank(chrome);
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  if (gesture === "gap") hitBlank(panel);
  else if (gesture === "Shift+Arrow") press("ArrowUp", { shiftKey: true });
  else {
    for (const type of ["pointerdown", "mousedown", "click"]) {
      cards[0].querySelector(".block-content-wrapper")!.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, shiftKey: true, detail: 1 }));
    }
  }
  expect(topSelected()).toEqual([]);
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(false);
  expect(pressAction(action).defaultPrevented).toBe(false);
  expect(topSelected()).toEqual([]);
  for (const block of flatten([first, second, ...target.blocks])) expect(node(block.id)?.raw).toBe(block.raw);
  expect(writeRich).not.toHaveBeenCalled();
});

it.each(removalKeys)("retires a blank selection when %s cannot remove its read-only roots", async action => {
  const routed = { ...target, read_only: true };
  const { root, writeRich } = await mountTarget(sources, groups, ["Alpha"], routed);
  hitBlank(root.querySelector(".page-blocks")!);
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(topSelected()).toEqual([routed.blocks[0].id]);
  expect(pressAction(action).defaultPrevented).toBe(true);
  await vi.waitFor(() => expect(topSelected()).toEqual([]));
  expect(pressAction("Delete").defaultPrevented).toBe(false);
  expect(node(routed.blocks[0].id)?.raw).toBe(routed.blocks[0].raw);
  if (action === "Cut") expectClipboard(writeRich.mock.lastCall!, routed.blocks);
  else expect(writeRich).not.toHaveBeenCalled();
});

it.each(["Delete", "Backspace"])("retires a blank selection on %s after its roots disappear", async action => {
  const { liveRow } = await mountTarget(sources, groups, ["Alpha", "Beta"]);
  const card = await vi.waitFor(() => {
    const row = liveRow(coldHit.id);
    expect(row).toBeTruthy();
    return row!.closest(".reference-group")!;
  });
  hitBlank(card);
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(topSelected()).toEqual([coldHit.id]);
  deleteBlock(coldHit.id);
  expect(node(coldHit.id)).toBeUndefined();
  expect(blankOutlineSelection()).toBeTruthy();
  expect(pressAction(action).defaultPrevented).toBe(true);
  expect(blankOutlineSelection()).toBeNull();
  expect(pressAction("Delete").defaultPrevented).toBe(false);
  for (const row of [hit, sources[1].blocks[1], target.blocks[0]]) expect(node(row.id)?.raw).toBe(row.raw);
});

it.each(["Copy", ...removalKeys])("selects the standalone live group from its body before %s", async action => {
  const { root, liveRow, writeRich } = await mountTarget();
  const portal = root.appendChild(document.createElement("div"));
  disposers.push(render(() => <LiveRefGroup {...groups[1]} surface="ref" eager />, portal));
  const body = await vi.waitFor(() => {
    expect(liveRow(coldHit.id)).toBeTruthy();
    return portal.querySelector(".page-blocks")!;
  });
  hitBlank(body);
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(topSelected()).toEqual([coldHit.id]);
  expect(pressAction(action).defaultPrevented).toBe(true);
  await vi.waitFor(() => expect(topSelected()).toEqual([]));
  if (action === "Copy" || action === "Cut") expectClipboard(writeRich.mock.lastCall!, [coldHit]);
  else expect(writeRich).not.toHaveBeenCalled();
  for (const block of flatten([coldHit])) expect(node(block.id)?.raw).toBe(action === "Copy" ? block.raw : undefined);
  for (const block of [hit, sources[1].blocks[1], target.blocks[0]]) expect(node(block.id)?.raw).toBe(block.raw);
});

it.each(blankSurfaces.flatMap(([surface, outline, id]) =>
  [".block-main", ".block-children-container > .block-children"].flatMap(zone =>
    ["Copy", ...removalKeys].map(action => ({ surface, outline, id, zone, action })),
  ),
))("row padding and child indentation restart selection in $surface ($zone, $action)", async ({ surface, outline, id, zone, action }) => {
  const targetId = "1d5ea000-0000-4000-a000-000000000001";
  const routed = page("Target", [
    block("target-root", `Target body\nid:: ${targetId}`, [block("target-child", "Body child")]),
    block("embed-host", "{{embed [[Embedded]]}}"),
    block("query-host", "{{query (task TODO)}}"),
  ]);
  const embedded = page("Embedded", [block("embedded-root", "Embedded body", [block("embedded-child", "Embed child")])]);
  const mention = block("unlinked-child", "A plain mention of Target");
  const unlinked = page("Unlinked", [block("unlinked-root", "Unlinked context", [mention])]);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([{
    page: unlinked.name, kind: unlinked.kind, path: unlinked.id, blocks: [shallow(mention)],
  }]);
  vi.spyOn(backend(), "getBlockReferrers").mockResolvedValue([groups[1]]);
  backendReadsQueries({ "(task TODO)": { form: "(task TODO)" } });
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([groups[1]]));
  const { root, writeRich } = await mountTarget([...sources, embedded, unlinked], groups, ["Alpha", "Beta", "Embedded", "Unlinked"], routed);
  if (surface === "Unlinked References") {
    root.querySelector<HTMLElement>(".unlinked-references .references-header")!.click();
    const crumb = await vi.waitFor(() => {
      const element = root.querySelector<HTMLElement>(".unlinked-references .ref-crumb[role=button]");
      expect(element).not.toBeNull();
      return element!;
    });
    crumb.click();
  }
  if (surface === "block references") requestBlockReferences(targetId);
  const alpha = root.querySelector<HTMLElement>(".linked-references .reference-group")!;
  const row = `${outline} [data-block-id="${id}"]`;
  const expected = id === "target-root" ? routed.blocks : id === "embedded-root" ? embedded.blocks
    : id === "unlinked-root" ? unlinked.blocks : [coldHit];
  const pointerTarget = await vi.waitFor(() => {
    const hit = root.querySelector(`${row} > ${zone}`);
    expect(hit).not.toBeNull();
    return hit!;
  });
  hitBlank(alpha);
  press("a", { ctrlKey: true });
  expect(selectedIds()).toContain("alpha-hit");
  hitBlank(pointerTarget);
  expect(selectedIds()).toEqual([]);
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(topSelected()).toEqual(expected.map(block => block.id));
  const selected = flatten(expected);
  expect(selectedIds()).toEqual(selected.map(block => block.id));
  const source = flatten([...routed.blocks, ...embedded.blocks, ...unlinked.blocks, ...sources.flatMap(page => page.blocks)])
    .filter(block => node(block.id));
  expect(pressAction(action).defaultPrevented).toBe(true);
  await vi.waitFor(() => expect(selectedIds()).toEqual([]));
  for (const block of source) {
    expect(node(block.id)?.raw, block.id).toBe(action !== "Copy" && selected.includes(block) ? undefined : block.raw);
  }
  if (action === "Copy" || action === "Cut") {
    expectClipboard(writeRich.mock.lastCall!, expected.map(block => block.id === "target-root" ? { ...block, raw: "Target body" } : block));
  } else expect(writeRich).not.toHaveBeenCalled();
});

function expectClipboard([plain, html]: [string, string], roots: BlockDto[]) {
  function markdown(blocks: BlockDto[], depth = 0): string[] {
    return blocks.flatMap(block => [`${"\t".repeat(depth)}- ${block.raw}`, ...markdown(block.children, depth + 1)]);
  }
  expect(plain).toBe(markdown(roots).join("\n"));
  const copied = new DOMParser().parseFromString(html, "text/html");
  function tree(list: Element | null): unknown[] {
    return [...(list?.children ?? [])].map(li => [li.firstChild?.textContent, tree(li.querySelector(":scope > ul"))]);
  }
  function expected(blocks: BlockDto[]): unknown[] {
    return blocks.map(block => [block.raw, expected(block.children)]);
  }
  expect(tree(copied.querySelector("body > ul"))).toEqual(expected(roots));
}

it.each(["Copy", ...removalKeys])("keeps the last folded root's subtree in blank selection before %s", async action => {
  const { liveRow, writeRich } = await mountTarget(sources, groups, ["Alpha", "Beta"]);
  const beta = await vi.waitFor(() => {
    const row = liveRow(coldHit.id);
    expect(row).not.toBeNull();
    return row!.closest<HTMLElement>(".reference-group")!;
  });
  expect(liveRow("beta-child")).not.toBeNull();
  liveRow(coldHit.id)!.querySelector<HTMLElement>(".collapse-toggle.has-children")!.click();
  expect(liveRow("beta-child")).toBeNull();
  beta.click();
  press("a", { ctrlKey: true });
  expect(topSelected()).toEqual([coldHit.id]);
  expect(pressAction(action).defaultPrevented).toBe(true);
  await vi.waitFor(() => expect(topSelected()).toEqual([]));
  if (action === "Copy" || action === "Cut") expectClipboard(writeRich.mock.lastCall!, [coldHit]);
  else expect(writeRich).not.toHaveBeenCalled();
  for (const row of flatten([coldHit])) expect(node(row.id)?.raw).toBe(action === "Copy" ? row.raw : undefined);
  for (const row of [hit, sources[1].blocks[1], target.blocks[0]]) expect(node(row.id)?.raw).toBe(row.raw);
});

it.each(["plain", "full text", "live breadcrumb", "direct Delete"])("leaves Unlinked Reference excerpts out of Copy and Delete (%s)", async mode => {
  const mention = block("mention", "Mention of Target");
  const excerpt = block("excerpt", "Another mention of Target", [block("hidden", "Undisplayed child")]);
  const parent = block("mention-parent", "Breadcrumb parent", [mention]);
  const unlinked = page("Unlinked", [parent, excerpt]);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([{
    page: unlinked.name, kind: unlinked.kind, path: unlinked.id, blocks: [shallow(mention), shallow(excerpt)],
  }]);
  const { root, liveRow, writeRich } = await mountTarget([...sources, unlinked], groups, ["Alpha", "Unlinked"]);
  root.querySelector<HTMLElement>(".unlinked-references .references-header")!.click();
  const group = await vi.waitFor(() => {
    const group = root.querySelector<HTMLElement>(".unlinked-references .reference-group");
    expect(group?.querySelector(".ref-crumb[role=button]")).not.toBeNull();
    expect(node(excerpt.id)).toBeDefined();
    return group!;
  });
  if (mode === "full text") {
    group.querySelector<HTMLElement>('[data-reference-block="excerpt"] .reference-show-full')!.click();
  }
  if (mode === "live breadcrumb") {
    group.querySelector<HTMLElement>(".ref-crumb[role=button]")!.click();
    expect(liveRow(parent.id)).not.toBeNull();
  }
  hitBlank(group);
  press("a", { ctrlKey: true });
  if (mode === "live breadcrumb") {
    press("c", { ctrlKey: true });
    expectClipboard(writeRich.mock.lastCall!, [parent]);
    hitBlank(group);
    press("a", { ctrlKey: true });
  } else if (mode !== "direct Delete") {
    expect(press("c", { ctrlKey: true }).defaultPrevented).toBe(false);
    expect(writeRich).not.toHaveBeenCalled();
  }
  press("Delete");
  expect(node(excerpt.id)?.raw).toBe(excerpt.raw);
  expect(node("hidden")?.raw).toBe("Undisplayed child");
  expect(node(hit.id)?.raw).toBe(hit.raw);
  expect(!!node(parent.id)).toBe(mode !== "live breadcrumb");
});

it.each(["Delete", "Backspace", "Cut"])("leaves the undisplayed parent and sibling intact after %s then Delete", async gesture => {
  const sibling = block("beta-unrelated", "Other child of the source parent");
  const parent = block("beta-parent", "Unselected parent", [coldHit, sibling]);
  const { root } = await mountTarget(
    [sources[0], { ...sources[1], blocks: [parent] }, sources[2]], groups, ["Alpha", "Beta"],
  );
  const group = await vi.waitFor(() => {
    const row = root.querySelector('.ls-block:not(.ref-block)[data-block-id="beta-hit"] > .block-main');
    expect(row).not.toBeNull();
    return row!.closest(".reference-group")!;
  });
  hitBlank(group);
  press("a", { ctrlKey: true });
  expect(selectedIds()).toEqual([coldHit.id, coldHit.children[0].id, coldHit.children[0].children[0].id]);
  expect((gesture === "Cut" ? press("x", { ctrlKey: true }) : press(gesture)).defaultPrevented).toBe(true);
  await vi.waitFor(() => expect(node(coldHit.id)).toBeUndefined());
  expect(selectedIds()).toEqual([]);
  expect(press("Delete").defaultPrevented).toBe(false);
  expect(node(parent.id)?.children).toEqual([sibling.id]);
  expect(node(sibling.id)?.raw).toBe(sibling.raw);
  expect(node(hit.id)?.raw).toBe(hit.raw);
  expect(node(target.blocks[0].id)?.raw).toBe(target.blocks[0].raw);
});

it.each([
  ["nested zoom", ".zoomed-page", "Delete"],
  ["nested block embed", ".embed-block", "Backspace"],
  ["child-only Linked References", ".linked-references", "Cut"],
  ["child-only Unlinked References", ".unlinked-references", "Delete"],
  ["child-only block references", ".block-references", "Backspace"],
  ["child-only query results", ".query-group", "Cut"],
])("reselects the surviving parent after editor Ctrl+A in %s (%s, %s)", async (surface, selector, action) => {
  const uuid = "1d5ea000-0000-4000-a000-000000000002";
  const leaf = block("scope-leaf", "Last scoped block");
  const tree = block("scope-root", surface === "nested block embed" ? `Scoped tree\nid:: ${uuid}` : "Scoped tree", [leaf]);
  const sibling = block("scope-sibling", "Another branch");
  const parent = block("scope-parent", "Source parent", [tree, sibling]);
  const source = page("Source", [parent]);
  const result = { page: source.name, kind: source.kind, path: source.id, blocks: [shallow(tree)] };
  const routed = { ...target, blocks: [...target.blocks] };
  if (surface === "nested zoom") routed.blocks.push(parent);
  if (surface === "nested block embed") {
    routed.blocks.push(block("embed-host", `{{embed ((${uuid}))}}`));
    vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(id => id === uuid ? result : null));
  }
  if (surface === "child-only block references") vi.spyOn(backend(), "getBlockReferrers").mockResolvedValue([result]);
  if (surface === "child-only Unlinked References") {
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([{ ...result, blocks: [shallow(leaf)] }]);
  }
  if (surface === "child-only query results") {
    routed.blocks.push(block("query-host", "{{query (task TODO)}}"));
    backendReadsQueries({ "(task TODO)": { form: "(task TODO)" } });
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([result]));
  }
  const linked = surface === "child-only Linked References" ? [...groups, result] : groups;
  const { root, writeRich } = await mountTarget([...sources, source], linked, ["Alpha", "Source"], routed);
  if (surface === "nested zoom") mainPaneRouter.zoomToBlock({ name: routed.name, pageKind: "page", block: tree.id });
  if (surface === "child-only block references") requestBlockReferences(target.blocks[0].id);
  if (surface === "child-only Unlinked References") {
    root.querySelector<HTMLElement>(`${selector} .references-header`)!.click();
    const crumb = await vi.waitFor(() => {
      const crumb = [...root.querySelectorAll<HTMLElement>(`${selector} .ref-crumb[role=button]`)]
        .find(crumb => crumb.textContent === tree.raw);
      expect(crumb).toBeDefined();
      return crumb!;
    });
    crumb.click();
  }
  const outline = await vi.waitFor(() => {
    const outline = root.querySelector(selector);
    expect(outline?.querySelector(`.ls-block:not(.ref-block)[data-block-id="${leaf.id}"] > .block-main`)).toBeTruthy();
    return outline!;
  });
  const editor = await openBlockEditor(outline, leaf.id);
  editor.keydown({ key: "a", ctrlKey: true });
  // jsdom has no native textarea select-all default action.
  if (editor.kind === "textarea") editor.selectByUser(0, editor.value().length);
  expect(editor.keydown({ key: "a", ctrlKey: true }).defaultPrevented).toBe(true);
  expect(selectedIds()).toEqual([leaf.id]);
  for (let step = 0; step < 2; step++) expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(topSelected()).toEqual([tree.id]);
  expect(selectedIds()).toEqual([tree.id, leaf.id]);
  expect(pressAction("Copy").defaultPrevented).toBe(true);
  expectClipboard(writeRich.mock.lastCall!, [{ ...tree, raw: "Scoped tree" }]);
  expect((action === "Cut" ? press("x", { ctrlKey: true }) : press(action)).defaultPrevented).toBe(true);
  await vi.waitFor(() => expect(node(tree.id)).toBeUndefined());
  if (action === "Cut") expectClipboard(writeRich.mock.lastCall!, [tree]);
  expect(node(leaf.id)).toBeUndefined();
  expect(node(parent.id)?.children).toEqual([sibling.id]);
  expect(node(target.blocks[0].id)?.raw).toBe(target.blocks[0].raw);
  expect(selectedIds()).toEqual([parent.id, sibling.id]);
});

it.each(removalKeys)("ends an earlier selection when a blank hit has no live rows, before %s", async action => {
  const { root, writeRich } = await mountTarget();
  const body = await openBlockEditor(root, target.blocks[0].id);
  body.keydown({ key: "Escape" });
  expect(selectedIds()).toEqual([target.blocks[0].id]);
  const cuts = vi.spyOn(cut, "cutBlocks");
  hitBlank(root.querySelector(".linked-references .reference-group:last-of-type")!);
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(false);
  expect(selectedIds()).toEqual([]);
  pressAction(action);
  await cuts.mock.results.at(-1)?.value;
  expect(node(target.blocks[0].id)?.raw).toBe(target.blocks[0].raw);
  expect(writeRich).not.toHaveBeenCalled();
});

it.each([
  ["from a button to the group header", ".reference-group-disclosure", ".reference-group-disclosure", ".reference-group-header", 0, false],
  ["from the header onto its button", ".reference-group-header", ".reference-group-disclosure", ".reference-group-header", 0, false],
  ["beyond the drag threshold and back", ".reference-group-header", ".reference-group-header", ".reference-group-header", 20, false],
  ["with Shift released before the click", ".reference-group-header", ".reference-group-header", ".reference-group-header", 0, true],
  ["captured by the header but released on its button", ".reference-group-header", "captured", ".reference-group-header", 0, false],
  ["captured by the header but released outside the view", ".reference-group-header", "outside", ".reference-group-header", 0, false],
] as const)("does not arm blank selection after a press %s", async (_, from, up, to, moved, shiftKey) => {
  const { root } = await mountTarget();
  const group = root.querySelector(".linked-references .reference-group")!;
  group.querySelector(from)!.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0, shiftKey }));
  if (moved) window.dispatchEvent(new MouseEvent("pointermove", { clientX: moved }));
  const button = group.querySelector(".reference-group-disclosure");
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => up === "captured" ? button : null });
  try {
    group.querySelector(up === "captured" || up === "outside" ? from : up)!.dispatchEvent(new MouseEvent("pointerup", { bubbles: true, button: 0 }));
  } finally {
    Reflect.deleteProperty(document, "elementFromPoint");
  }
  group.querySelector(to)!.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 }));
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(false);
  expect(selectedIds()).toEqual([]);
});

it("retires a blank selection when its Cut completes after a refresh changed it", async () => {
  const { root, writeRich } = await mountTarget();
  const write = Promise.withResolvers<void>();
  writeRich.mockReturnValueOnce(write.promise);
  const operation = vi.spyOn(cut, "cutBlocks");
  hitBlank(root.querySelector(".linked-references .reference-group")!);
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(press("x", { ctrlKey: true }).defaultPrevented).toBe(true);
  deleteBlock(nestedHit.id);
  write.resolve();
  await operation.mock.results[0].value;
  await vi.waitFor(() => expect(blankOutlineSelection()).toBeNull());
  expect(node(hit.id)?.raw).toBe(hit.raw);
});

it("does not let an empty Cut remove a later selection", async () => {
  const { root, liveRow, writeRich } = await mountTarget(sources, groups, ["Alpha", "Beta"]);
  const card = await vi.waitFor(() => liveRow(coldHit.id)!.closest(".reference-group")!);
  hitBlank(card);
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  deleteBlock(coldHit.id);
  const cuts = vi.spyOn(cut, "cutBlocks");
  press("x", { ctrlKey: true });
  const editor = await openBlockEditor(root, target.blocks[0].id);
  editor.keydown({ key: "Escape" });
  expect(selectedIds()).toEqual([target.blocks[0].id]);
  await cuts.mock.results.at(-1)?.value;
  expect(node(target.blocks[0].id)?.raw).toBe(target.blocks[0].raw);
  expect(selectedIds()).toEqual([target.blocks[0].id]);
  expect(writeRich).not.toHaveBeenCalled();
});

it("does not let a pending ordinary Cut remove a later blank selection of the same blocks", async () => {
  const { root, writeRich } = await mountTarget();
  const write = Promise.withResolvers<void>();
  writeRich.mockReturnValueOnce(write.promise);
  const operation = vi.spyOn(cut, "cutBlocks");
  const editor = await openBlockEditor(root, hit.id);
  editor.keydown({ key: "Escape" });
  extendSelectionTo(nestedHit.id);
  const ids = selectedIds();
  expect(press("x", { ctrlKey: true }).defaultPrevented).toBe(true);
  hitBlank(root.querySelector(".linked-references .reference-group")!);
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(selectedIds()).toEqual(ids);
  write.resolve();
  await operation.mock.results[0].value;
  expect(node(hit.id)?.raw).toBe(hit.raw);
  expect(node(nestedHit.id)?.raw).toBe(nestedHit.raw);
});

it("leaves a pending Cut as a copy after the same group is selected again", async () => {
  const { root, writeRich } = await mountTarget();
  const write = Promise.withResolvers<void>();
  writeRich.mockReturnValueOnce(write.promise);
  const operation = vi.spyOn(cut, "cutBlocks");
  const group = root.querySelector(".linked-references .reference-group")!;
  hitBlank(group);
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(press("x", { ctrlKey: true }).defaultPrevented).toBe(true);
  hitBlank(group);
  expect(selectedIds()).toEqual([]);
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  write.resolve();
  await operation.mock.results[0].value;
  expectClipboard(writeRich.mock.lastCall!, [hit, nestedHit]);
  expect(selectedIds()).toContain(hit.id);
  expect(node(hit.id)?.raw).toBe(hit.raw);
  expect(node(nestedHit.id)?.raw).toBe(nestedHit.raw);
  expect(node(target.blocks[0].id)?.raw).toBe(target.blocks[0].raw);
});

it("completes a pending blank Cut when its group's outline is recomputed", async () => {
  const { root, writeRich } = await mountTarget();
  const write = Promise.withResolvers<void>();
  writeRich.mockReturnValueOnce(write.promise);
  const operation = vi.spyOn(cut, "cutBlocks");
  hitBlank(root.querySelector(".linked-references .reference-group")!);
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(press("x", { ctrlKey: true }).defaultPrevented).toBe(true);
  const before = topSelected();
  setDoc("byId", hit.id, "collapsed", true); // same roots, recomputed topology
  expect(topSelected()).not.toBe(before);
  write.resolve();
  await operation.mock.results[0].value;
  expectClipboard(writeRich.mock.lastCall!, [hit, nestedHit]);
  expect(node(hit.id)).toBeUndefined();
  expect(node(nestedHit.id)).toBeUndefined();
});

it("uses the live reference outline after a blank hit on its cold shell", async () => {
  const { root, liveRow, writeRich, showSource } = await mountTarget();
  const beta = root.querySelectorAll(".linked-references .reference-group")[1];
  hitBlank(beta);
  expect(liveRow(coldHit.id)).toBeNull();
  showSource("Beta");
  await vi.waitFor(() => expect(liveRow(coldHit.id)).not.toBeNull());
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  press("c", { ctrlKey: true });
  expectClipboard(writeRich.mock.lastCall!, [coldHit]);
});

it.each(["keyboard", "Shift-click", "drag"])("keeps the full reference range after Ctrl+A and a fold (%s)", async gesture => {
  const first = block("beta-first", "First Beta hit [[Target]]");
  const last = { ...coldHit, children: [...coldHit.children, block("beta-tail", "Beta tail")] };
  const { liveRow, writeRich } = await mountTarget(
    [sources[0], { ...sources[1], blocks: [first, last] }, sources[2]],
    [groups[0], { ...groups[1], blocks: [shallow(first), shallow(last)] }, groups[2]],
    ["Alpha", "Beta"],
  );
  await vi.waitFor(() => expect(liveRow("beta-child")).not.toBeNull());
  const pressRow = (id: string, shiftKey = false) =>
    liveRow(id)!.querySelector(".block-content-wrapper")!.dispatchEvent(new MouseEvent("mousedown", {
      bubbles: true, cancelable: true, shiftKey, clientX: 30, clientY: 30,
    }));
  if (gesture === "keyboard") {
    pressRow("beta-tail");
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    press("Escape", {}, document.activeElement!);
    for (let step = 0; step < 3; step++) expect(press("ArrowUp", { shiftKey: true }).defaultPrevented).toBe(true);
  } else if (gesture === "drag") {
    pressRow(first.id);
    Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => liveRow("beta-tail") });
    try {
      document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: 30, clientY: 90 }));
    } finally {
      document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
      Reflect.deleteProperty(document, "elementFromPoint");
    }
  } else {
    pressRow(first.id);
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    pressRow("beta-tail", true);
  }
  expect(selectedIds()).toEqual([first.id, coldHit.id, "beta-child", "beta-grandchild", "beta-tail"]);
  press("a", { ctrlKey: true });
  liveRow(coldHit.id)!.querySelector<HTMLElement>(".collapse-toggle.has-children")!.click();
  expect(liveRow("beta-child")).toBeNull();
  press("c", { ctrlKey: true });
  expectClipboard(writeRich.mock.lastCall!, [first, last]);
});

it("copies the clicked comment group on a routed page", async () => {
  const comment = (id: string, text: string, children: BlockDto[] = []) => ({
    ...block(id, `${text}\ntine.quote:: words`, children), properties: [["tine.quote", "words"]] as [string, string][],
  });
  const first = comment("thread-1", "First comment");
  const second = comment("thread-2", "Second comment", [block("reply-2", "Nested reply")]);
  const third = comment("thread-3", "Third comment");
  const { root, writeRich } = await mountTarget(sources, groups, ["Alpha"],
    page("Target", [block("passage", "Some words here", [first, second, third])]));
  openCommentThread(root);
  const group = await vi.waitFor(() => {
    const group = document.querySelector(".comment-group");
    expect(group?.querySelector('[data-block-id="thread-3"]')).not.toBeNull();
    return group!;
  });
  hitBlank(group);
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(topSelected()).toEqual([first.id, second.id, third.id]);
  expect(selectedIds()).toEqual([first.id, second.id, "reply-2", third.id]);
  expect(press("c", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(writeRich.mock.lastCall?.[0]).toContain("\t- Nested reply");
  const html = new DOMParser().parseFromString(writeRich.mock.lastCall![1], "text/html");
  expect(html.querySelector("body > ul > li:nth-child(2) > ul > li")?.textContent).toBe("Nested reply");
});
