import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { clearClipboardPayload } from "../clipboard";
import { node, resetStore, selectedIds } from "../document";
import { endEdit } from "../editorController";
import { setGraphMeta } from "../graphSession";
import { installKeybindings } from "../keybindings";
import { resetNearObserverForTests } from "../lazyObserve";
import { resetReferenceSectionState } from "../referenceSectionState";
import { initParser } from "../render/parse";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { maybeEditorIn } from "../tests/editorDriver";
import { backendReadsQueries, blockRunResult } from "../tests/queryReadingsTestkit";
import type { BlockDto, PageRead, RefGroup } from "../types";
import { closeExportModal, requestBlockReferences, setBlockReferencesRequest } from "../ui";
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
  setBlockReferencesRequest(null);
  clearClipboardPayload();
  resetStore();
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
  vi.stubGlobal("IntersectionObserver", class {
    constructor(private callback: IntersectionObserverCallback) {}
    observe(target: Element) {
      const group = target.closest(".reference-group");
      const isIntersecting = !group || visibleSources.includes(group.querySelector(".reference-page")?.textContent ?? "");
      queueMicrotask(() => this.callback([{ target, isIntersecting } as IntersectionObserverEntry], this as unknown as IntersectionObserver));
    }
    unobserve() {}
    disconnect() {}
  });
  vi.spyOn(backend(), "getPage").mockImplementation(async name => name === routedPage.name ? routedPage : sourcePages.find(source => source.name === name) ?? null);
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

function press(key: string, options: KeyboardEventInit = {}, target: Element = document.body) {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...options });
  target.dispatchEvent(event);
  return event;
}

async function openAlphaEditor(root: HTMLElement) {
  const content = root.querySelector<HTMLElement>(`[data-block-id="${hit.id}"] > .block-main .block-content`)!;
  content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  return vi.waitFor(() => {
    const editor = maybeEditorIn(root, hit.id);
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
  const editor = await openAlphaEditor(root);
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
  const mouseEditor = await openAlphaEditor(root);
  mouseEditor.keydown({ key: "Escape" });
  expect(press("c", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(writeRich).toHaveBeenCalledOnce();
  const singleTree = writeRich.mock.calls[0];

  const editor = await openAlphaEditor(root);
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

it("copies trees from reference whitespace and widens only to loaded groups", async () => {
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
  const [alphaGroup, betaGroup, gammaGroup] = root.querySelectorAll<HTMLElement>(".reference-group");
  root.tabIndex = -1;
  root.focus(); // The browser gives the clicked group's pane scroller keyboard focus.
  alphaGroup.click();
  const alpha = "- Alpha reference [[Target]]\n\t- Alpha child\n\t\t- Alpha grandchild\n- Other Alpha reference [[Target]]\n\t- Other Alpha child";
  const beta = "- Beta reference [[Target]]\n\t- Beta child\n\t\t- Beta grandchild";
  for (const [index, plain] of [alpha, [alpha, beta].join("\n"), [alpha, beta].join("\n")].entries()) {
    expect(press("a", { ctrlKey: true }, root).defaultPrevented, `select-all press ${index + 1}`).toBe(true);
    expect(press("c", { ctrlKey: true }, root).defaultPrevented).toBe(true);
    expect(writeRich).toHaveBeenCalledTimes(index + 1);
    expect(writeRich.mock.calls[index][0]).toBe(plain);
    const html = new DOMParser().parseFromString(writeRich.mock.calls[index][1], "text/html");
    const grandchildren = [...html.querySelectorAll("body > ul > li > ul > li > ul > li")].map(li => li.textContent);
    expect(grandchildren).toEqual(index === 0 ? ["Alpha grandchild"] : ["Alpha grandchild", "Beta grandchild"]);
  }
  betaGroup.querySelector<HTMLButtonElement>(".reference-group-disclosure")!.click();
  expect(press("c", { ctrlKey: true }).defaultPrevented).toBe(false);
  gammaGroup.click();
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(press("c", { ctrlKey: true }).defaultPrevented).toBe(false);
  expect(readByPath).toHaveBeenCalledTimes(reads);
  expect(submit).not.toHaveBeenCalled();
});

it("selects the routed page outline from blank space and preserves native select-all in fields", async () => {
  const { root, writeRich } = await mountTarget();
  root.querySelector<HTMLElement>(".page-blocks")!.click();
  for (const field of [document.createElement("input"), document.createElement("textarea"), document.createElement("div")]) {
    if (field.tagName === "DIV") field.setAttribute("contenteditable", "true");
    root.append(field);
    field.focus();
    expect(press("a", { ctrlKey: true }, field).defaultPrevented).toBe(false);
    field.remove();
  }
  expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
  const dialog = document.createElement("div");
  dialog.setAttribute("role", "dialog");
  root.append(dialog);
  expect(press("a", { ctrlKey: true }, dialog).defaultPrevented).toBe(false);
  expect(press("c", { ctrlKey: true }).defaultPrevented).toBe(true);
  expect(writeRich.mock.calls[0][0]).toBe("- Target page body");
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
  for (const roots of [expected, [...expected, coldHit.raw]]) {
    expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
    expect(press("c", { ctrlKey: true }).defaultPrevented).toBe(true);
    const [plain, html] = writeRich.mock.lastCall!;
    expect(plain.split("\n").filter(line => line.startsWith("- "))).toEqual(roots.map(raw => `- ${raw}`));
    expect(plain).toContain(`\n\t- ${hit.children[0].raw}`);
    const copied = new DOMParser().parseFromString(html, "text/html");
    expect([...copied.querySelectorAll("body > ul > li")].map(li => li.firstChild?.textContent)).toEqual(roots);
  }
});

it.each(["group to group", "Linked to Unlinked", "page to embed"])("starts in the newly clicked outline from %s", async (journey) => {
  const embedded = page("Embedded", [block("embedded-root", "Embedded outline")]);
  const embedHost = block("embed-host", "{{embed [[Embedded]]}}");
  const withEmbed = { ...target, blocks: [...target.blocks, embedHost] };
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([{ ...groups[2], blocks: [shallow(excludedHit)] }]);
  const { root, liveRow, writeRich } = await mountTarget([...sources, embedded], groups, ["Alpha", "Beta", "Gamma"], withEmbed);
  const [alpha, beta] = root.querySelectorAll<HTMLElement>(".linked-references .reference-group");
  await vi.waitFor(() => expect(liveRow(coldHit.id)).not.toBeNull());
  let from = alpha, to = beta, first = coldHit.raw;
  let expected = [coldHit.id, coldHit.children[0].id, coldHit.children[0].children[0].id];
  if (journey === "Linked to Unlinked") {
    root.querySelector<HTMLElement>(".unlinked-references .references-header")!.click();
    to = await vi.waitFor(() => {
      const group = root.querySelector<HTMLElement>(".unlinked-references .reference-group");
      expect(group?.textContent).toContain(excludedHit.raw);
      return group!;
    });
    first = excludedHit.raw;
    expected = [excludedHit.id];
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
  at.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 }));
}

const blankSurfaces = [
  ["Linked References", ".linked-references", coldHit.id],
  ["Unlinked References", ".unlinked-references", "unlinked-root"],
  ["embed", ".embed-block", "embedded-root"],
  ["page body", ".page-blocks", "target-root"],
  ["query results", ".query-group", coldHit.id],
  ["block references", ".block-references-inner", coldHit.id],
] as const;

it.each(blankSurfaces.flatMap(([surface, outline, id]) => [
  { surface, outline, id, zone: ".block-main" },
  { surface, outline, id, zone: ".block-children-container > .block-children" },
]))("row padding and child indentation restart selection in $surface ($zone)", async ({ surface, outline, id, zone }) => {
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
  const { root } = await mountTarget([...sources, embedded, unlinked], groups, ["Alpha", "Beta", "Embedded", "Unlinked"], routed);
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
  const expected = id === "target-root" ? ["target-root", "target-child", "embed-host", "query-host"]
    : id === "embedded-root" ? ["embedded-root", "embedded-child"]
    : id === "unlinked-root" ? ["unlinked-root", "unlinked-child"] : ["beta-hit", "beta-child", "beta-grandchild"];
  const pointerTarget = await vi.waitFor(() => {
    const hit = root.querySelector(`${row} > ${zone}`);
    expect(hit).not.toBeNull();
    return hit!;
  });
  hitBlank(alpha);
  press("a", { ctrlKey: true });
  expect(selectedIds()).toContain("alpha-hit");
  hitBlank(pointerTarget);
  if (surface !== "Linked References" || zone !== ".block-main") {
    expect(selectedIds()).toEqual([]);
    expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
    expect(selectedIds()).toEqual(expected);
  } else {
    press("a", { ctrlKey: true });
    expect(press("Delete").defaultPrevented).toBe(true);
    expect(node("alpha-hit")?.raw).toBe(hit.raw);
    expect(node("alpha-child")?.raw).toBe("Alpha child");
    expect(node("beta-hit")).toBeUndefined();
    expect(node("beta-child")).toBeUndefined();
  }
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

it.each([false, true])("copies the last folded root's subtree after select all (widened: %s)", async widened => {
  const { root, liveRow, writeRich } = await mountTarget(sources, groups, ["Alpha", "Beta"]);
  const beta = await vi.waitFor(() => {
    const row = liveRow(coldHit.id);
    expect(row).not.toBeNull();
    return row!.closest<HTMLElement>(".reference-group")!;
  });
  const start = widened ? root.querySelector<HTMLElement>(".linked-references .reference-group")! : beta;
  start.click();
  press("a", { ctrlKey: true });
  if (widened) press("a", { ctrlKey: true });
  expect(liveRow("beta-child")).not.toBeNull();
  liveRow(coldHit.id)!.querySelector<HTMLElement>(".collapse-toggle.has-children")!.click();
  expect(liveRow("beta-child")).toBeNull();
  expect(press("c", { ctrlKey: true }).defaultPrevented).toBe(true);
  expectClipboard(writeRich.mock.lastCall!, widened ? [hit, nestedHit, coldHit] : [coldHit]);
});

it("steps back through both ends of a selection whose breadcrumb repeats a hit", async () => {
  const q = block("alpha-q", "Second hit [[Target]]");
  const r = block("alpha-r", "Third hit [[Target]]");
  const { root, writeRich } = await mountTarget(
    [{ ...sources[0], blocks: [hit, q, r] }, ...sources.slice(1)],
    [{ ...groups[0], blocks: [hit, q, r, hit.children[0]].map(shallow) }, ...groups.slice(1)],
  );
  const alpha = root.querySelector<HTMLElement>(".linked-references .reference-group")!;
  alpha.querySelector<HTMLElement>(".ref-crumb[role=button]")!.click();
  expect(alpha.querySelectorAll(`[data-block-id="${hit.id}"]`)).toHaveLength(2);
  alpha.click();
  press("a", { ctrlKey: true });
  for (const [key, roots] of [
    ["ArrowUp", [hit, q, r]], // The last visible child back to its repeated parent.
    ["ArrowUp", [hit, q, r]], // The repeated parent back to R.
    ["ArrowUp", [hit, q]],
    ["ArrowDown", [hit, q, r]], // Reverse back to R.
  ] as const) {
    expect(press(key, { shiftKey: true }).defaultPrevented).toBe(true);
    expect(press("c", { ctrlKey: true }).defaultPrevented).toBe(true);
    expectClipboard(writeRich.mock.lastCall!, [...roots]);
  }
});
