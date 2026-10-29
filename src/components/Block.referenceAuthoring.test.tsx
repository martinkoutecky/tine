import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { startEditing } from "../editorController";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc, freshId } from "../document/model";
import { initParser } from "../render/parse";
import { refreshPageIndex, resetPageIndex } from "../pageIndex";
import type { BlockDto, PageDto, PageEntry, PageRead } from "../types";
import { Block } from "./Block";
import { isConflicted } from "../document/host/wiring";
import { bindTestHost, submittedPages } from "../document/host/wiring.test.support";
import { openAsLoaded } from "./hostConflict.test.support";
import { setPinyinMatchingEnabled } from "../launcherRanking";
import { setActiveLocale } from "../i18n";

beforeAll(() => initParser());

const V4_UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

afterEach(() => {
  vi.restoreAllMocks();
  resetPageIndex();
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.append(root);
  return { root, dispose: render(node, root) };
}

function page(raw: string): PageDto {
  const block: BlockDto = { id: "reference-authoring", raw, collapsed: false, children: [] };
  return { name: "Reference authoring", kind: "page", title: "Reference authoring", pre_block: null, blocks: [block] };
}

function update(textarea: HTMLTextAreaElement) {
  textarea.focus();
  textarea.setSelectionRange(textarea.value.length, textarea.value.length);
  textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: textarea.value.at(-1) }));
}

function inputAt(textarea: HTMLTextAreaElement, value: string, caret: number) {
  textarea.focus();
  textarea.value = value;
  textarea.setSelectionRange(caret, caret);
  textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: value[caret - 1] ?? null }));
}

function entry(name: string): PageEntry {
  return { name, kind: "page", date_key: null, path: `pages/${name}.md` };
}

function accept(textarea: HTMLTextAreaElement, key: "Enter" | "Tab" = "Enter") {
  textarea.dispatchEvent(new KeyboardEvent("keydown", { key, code: key, bubbles: true, cancelable: true }));
}

describe("reference authoring", () => {
  it("keeps native rows ahead of Pinyin at the rendered completion boundary and refreshes when disabled", async () => {
    const lookup = vi.spyOn(backend(), "quickSwitch").mockImplementation(async (_query, limit, _editor, enabled) => limit === 0 ? [] : [
      { ...entry("x beijing"), match_class: "substring" },
      ...(enabled ? [{ ...entry("北京"), match_class: "transliteration" as const }] : []),
    ]);
    setPinyinMatchingEnabled(true);
    loadSingle(page(""));
    startEditing("reference-authoring", 0);
    const { root, dispose } = mount(() => <For each={pageByName("Reference authoring")?.roots ?? []}>{id => <Block id={id} />}</For>);
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "[[beijing", 9);
      const labels = () => [...document.querySelectorAll(".autocomplete .ac-label")].map(row => row.textContent);
      await vi.waitFor(() => expect(labels()).toContain("北京"));
      expect(labels().indexOf("x beijing")).toBeLessThan(labels().indexOf("北京"));
      const editor = lookup.mock.calls.find(call => call[0] === "beijing")![2]!;
      expect(editor.session).toBeTypeOf("string");
      setPinyinMatchingEnabled(false);
      await vi.waitFor(() => expect(labels()).not.toContain("北京"));
      expect(labels()).toContain("x beijing");
      expect(lookup).toHaveBeenCalledWith("", 0, editor, false);
    } finally { setPinyinMatchingEnabled(false); dispose(); }
  });

  it("releases baseline keys again when an obsolete native request replies after completion closes", async () => {
    let finish: ((pages: PageEntry[]) => void) | undefined;
    const lookup = vi.spyOn(backend(), "quickSwitch").mockImplementation(async (_query, limit) => {
      if (limit === 0) return [];
      return new Promise<PageEntry[]>(resolve => { finish = resolve; });
    });
    setPinyinMatchingEnabled(true);
    loadSingle(page(""));
    startEditing("reference-authoring", 0);
    const { root, dispose } = mount(() => <For each={pageByName("Reference authoring")?.roots ?? []}>{id => <Block id={id} />}</For>);
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "[[bj", 4);
      await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
      const editor = lookup.mock.calls.find(call => call[0] === "bj")![2]!;
      inputAt(textarea, "plain", 5);
      expect(lookup).toHaveBeenCalledWith("", 0, editor, false);
      finish!([entry("北京")]);
      await vi.waitFor(() => expect(lookup.mock.calls.filter(call => call[1] === 0)).toHaveLength(2));
      expect(document.querySelector(".autocomplete")).toBeNull();
    } finally { setPinyinMatchingEnabled(false); dispose(); }
  });
  it.each(["page", "tag"] as const)("keeps Create active when autosave echoes the new %s reference", async (kind) => {
    const quickSwitch = vi.spyOn(backend(), "quickSwitch").mockImplementation(async (_query, _limit, editor) => {
      // The native store probe covers this projection. At the UI boundary,
      // emulate the autosaved self-reference when no baseline is supplied.
      return editor?.originalRaw === "" ? [entry("Fuzzy Existing")] : [
        { ...entry("Fz"), path: "" }, entry("Fuzzy Existing"),
      ];
    });
    loadSingle(page(""));
    startEditing("reference-authoring", 0);
    const { root, dispose } = mount(() => <For each={pageByName("Reference authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>);
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, kind === "page" ? "[[Fz]]" : "#Fz", kind === "page" ? 4 : 3);
      const expected = kind === "page" ? 'Create "Fz"' : "Create #Fz";
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe(expected));
      expect(quickSwitch).toHaveBeenLastCalledWith("Fz", 100, { pageName: "Reference authoring", blockId: "reference-authoring", originalRaw: "" }, false);
      accept(textarea);
      expect(textarea.value.trim()).toBe(kind === "page" ? "[[Fz]]" : "#Fz");
    } finally { dispose(); }
  });

  it.each(["page", "tag"] as const)("displays and inserts authored aliases beside their matching owner in %s completion (GH #675, GH #676)", async (kind) => {
    vi.spyOn(backend(), "quickSwitch").mockResolvedValue([
      entry("Tine"),
      { ...entry("Tine"), name: "TineOutline" },
      { ...entry("Tine"), name: "Tine Outliner" },
    ]);
    vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: [
      { key: "tine", name: "Tine", is_journal: false, day: null, target: { kind: "existing", id: "pages/Tine.md", others: [] } },
      ...["TineOutline", "Tine Outliner"].map((name) => ({ key: name.toLowerCase(), name, is_journal: false, day: null, target: { kind: "alias" as const, owners: ["pages/Tine.md"] } })),
    ] });
    await refreshPageIndex();
    const input = kind === "page" ? "[[tin" : "#tin";
    loadSingle(page(input));
    startEditing("reference-authoring", input.length);
    const { root, dispose } = mount(() => <For each={pageByName("Reference authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>);
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, input, input.length);
      const labels = () => [...document.body.querySelectorAll(".autocomplete .ac-label")].map((row) => row.textContent);
      const prefix = kind === "tag" ? "#" : "";
      await vi.waitFor(() => expect(labels()).toContain(`${prefix}TineOutline`));
      expect(labels().filter((label) => !label?.startsWith("Create"))).toEqual([`${prefix}Tine`, `${prefix}TineOutline`, `${prefix}Tine Outliner`]);
      const row = [...document.body.querySelectorAll(".autocomplete .ac-label")].find((row) => row.textContent === `${prefix}TineOutline`)!.parentElement!;
      expect(row.querySelector(".ac-sub")?.textContent).toBe("alias of Tine");
      try {
        await setActiveLocale("zh-CN");
        expect(row.querySelector(".ac-sub")?.textContent).toBe("Tine 的别名");
        expect(labels()).toContain(`${prefix}TineOutline`);
        await setActiveLocale("en");
        expect(row.querySelector(".ac-sub")?.textContent).toBe("alias of Tine");
      } finally { await setActiveLocale("en"); }
      if (kind === "tag") {
        const index = labels().indexOf(`${prefix}TineOutline`);
        for (let i = 0; i < index; i++) textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", code: "ArrowDown", bubbles: true, cancelable: true }));
        accept(textarea);
      } else {
        row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
      }
      await vi.waitFor(() => expect(textarea.value.trim()).toBe(kind === "page" ? "[[TineOutline]]" : "#TineOutline"));
    } finally {
      dispose();
    }
  });

  it("renders PDF annotation navigation with the authored highlight id", () => {
    const authoredId = "7b6704f8-a337-4336-a711-2ba6bc14fbf2";
    loadSingle({
      name: "hls__paper",
      kind: "page",
      title: "hls__paper",
      pre_block: "file-path:: ../assets/paper.pdf",
      blocks: [{
        id: "runtime-annotation-id",
        raw: `Highlighted text\nid:: ${authoredId}\nls-type:: annotation\nhl-page:: 2\nhl-color:: yellow`,
        collapsed: false,
        children: [],
        properties: [
          ["id", authoredId],
          ["ls-type", "annotation"],
          ["hl-page", "2"],
          ["hl-color", "yellow"],
        ],
      }],
    });
    const { root, dispose } = mount(() => (
      <For each={pageByName("hls__paper")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      expect(root.querySelector(".hl-prefix")?.getAttribute("data-highlight-id")).toBe(authoredId);
    } finally {
      dispose();
    }
  });

  it("makes Page reference the active bare slash command and chains into a blank page lifecycle", async () => {
    loadSingle(page("/"));
    startEditing("reference-authoring", 1);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Reference authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      update(textarea);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("Page reference"));
      accept(textarea);
      await vi.waitFor(() => expect(doc.byId["reference-authoring"].raw).toBe("[[]]"));
      expect(textarea.selectionStart).toBe(2);
      expect(textarea.selectionEnd).toBe(2);
      expect(document.body.querySelector(".autocomplete")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("accepts a visible page row against the current trigger span before the debounced refresh", async () => {
    const quickSwitch = vi.spyOn(backend(), "quickSwitch").mockResolvedValue([entry("Parity Target")]);
    loadSingle(page("[[P]]"));
    startEditing("reference-authoring", 3);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Reference authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "[[P]]", 3);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("Parity Target"));
      expect(quickSwitch).toHaveBeenLastCalledWith("P", 100, { pageName: "Reference authoring", blockId: "reference-authoring", originalRaw: "[[P]]" }, false);

      // Type the rest and accept immediately, before the 90 ms backend debounce.
      // The visible row is still legitimate, but replacement must use the live
      // `[[Parity Tar` span rather than the stale `[[P` span.
      inputAt(textarea, "[[Parity Tar]]", 12);
      accept(textarea);

      await vi.waitFor(() => expect(doc.byId["reference-authoring"].raw).toBe("[[Parity Target]] "));
      expect(textarea.value).toBe("[[Parity Target]] ");
      expect(document.body.querySelector(".autocomplete")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("keeps other CJK page matches visible after the exact page without a Create row", async () => {
    vi.spyOn(backend(), "quickSwitch").mockResolvedValue([
      entry("我是小明"),
      entry("小明"),
      entry("小明的家"),
    ]);
    loadSingle(page("[[小明]]"));
    startEditing("reference-authoring", 4);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Reference authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "[[小明]]", 4);
      await vi.waitFor(() => expect(
        [...document.body.querySelectorAll(".autocomplete .ac-label")].map((row) => row.textContent),
      ).toEqual(["小明", "小明的家", "我是小明"]));
      expect(document.body.textContent).not.toContain('Create "小明"');
    } finally {
      dispose();
    }
  });

  it("requests OG's 20-result inline block-reference pool", async () => {
    const search = vi.spyOn(backend(), "search").mockResolvedValue([{
      page: "Source",
      kind: "page",
      blocks: [{ id: "needle-block", raw: "needle", collapsed: false, children: [] }],
      evidence: [],
    }]);
    loadSingle(page("((needle))"));
    startEditing("reference-authoring", 8);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Reference authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "((needle))", 8);
      await vi.waitFor(() => expect(search).toHaveBeenCalled());
      expect(search).toHaveBeenLastCalledWith("needle", 20, "block-picker");
    } finally {
      dispose();
    }
  });

  // An id-less target gets a fresh random UUID, never its runtime locator, even
  // a UUID-shaped one (GH #373, GH #798): `null` means "freshly minted".
  const blockReferenceCases: Array<[string, [string, string][] | undefined, string | null]> = [
    ["authored id", [["id", "11111111-1111-4111-8111-111111111111"]], "11111111-1111-4111-8111-111111111111"],
    ["authored non-UUID id", [["id", "my-own-id"]], "my-own-id"],
    ["id-less fallback", undefined, null],
  ];

  it.each(blockReferenceCases)("inserts the %s for an accepted block-reference suggestion", async (_case, properties, authoredId) => {
    vi.spyOn(backend(), "search").mockResolvedValue([{
      page: "Source",
      kind: "page",
      blocks: [{
        id: "f8358fac-56bd-8bb1-ba45-bd7fd1ba2add",
        raw: "test",
        collapsed: false,
        children: [],
        ...(properties ? { properties } : {}),
      }],
      evidence: [],
    }]);
    vi.spyOn(backend(), "getPage").mockResolvedValue({
      id: "pages/Source.md", name: "Source", kind: "page", title: "Source", pre_block: null,
      blocks: [{ id: "f8358fac-56bd-8bb1-ba45-bd7fd1ba2add", raw: properties ? `test\nid:: ${authoredId}` : "test", collapsed: false, children: [] }],
    });
    openAsLoaded(await bindTestHost());
    const save = vi.spyOn(backend(), "pageSubmit");
    loadSingle(page("((test"));
    startEditing("reference-authoring", 6);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Reference authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "((test", 6);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("test"));
      accept(textarea);
      await vi.waitFor(() => expect(doc.byId["reference-authoring"].raw).toMatch(/^\(\([^)]+\)\) $/));
      const inserted = doc.byId["reference-authoring"].raw.slice(2, -3);
      if (authoredId) expect(inserted).toBe(authoredId);
      else {
        expect(inserted).toMatch(V4_UUID);
        expect(inserted).not.toBe("f8358fac-56bd-8bb1-ba45-bd7fd1ba2add");
      }
      // The target's ID went to the host before the reference (STEP3 §8, Q2).
      if (!properties) expect(submittedPages(save, "Source")[0].blocks[0].raw).toBe(`test\nid:: ${inserted}`);
    } finally {
      dispose();
    }
  });

  it.each([
    ["block reference", "((test", (id: string) => `((${id})) `],
    ["block embed", "{{embed ((test", (id: string) => `{{embed ((${id})) `],
  ] as const)("never persists a block created this session by its runtime key from the %s picker (GH #798)", async (_case, typed, expected) => {
    // A block typed in this session keeps its frontend key (`b<base36>-<n>`) in
    // the backend too, so the block search returns that key as the row's id.
    const runtime = freshId();
    expect(runtime).toMatch(/^b[0-9a-z]+-\d+$/);
    const target: BlockDto = { id: runtime, raw: "test", collapsed: false, children: [] };
    vi.spyOn(backend(), "search").mockResolvedValue([{ page: "Source", kind: "page", blocks: [target], evidence: [] }]);
    vi.spyOn(backend(), "getPage").mockResolvedValue({
      id: "pages/Source.md", name: "Source", kind: "page", title: "Source", pre_block: null, blocks: [structuredClone(target)],
    });
    openAsLoaded(await bindTestHost());
    const save = vi.spyOn(backend(), "pageSubmit");
    loadSingle(page(typed));
    startEditing("reference-authoring", typed.length);
    const { root, dispose } = mount(() => <For each={pageByName("Reference authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>);
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, typed, typed.length);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("test"));
      accept(textarea);
      await vi.waitFor(() => expect(doc.byId["reference-authoring"].raw).not.toBe(typed));
      const written = submittedPages(save, "Source")[0].blocks[0].raw;
      const stamped = /^id:: (.+)$/m.exec(written)?.[1];
      expect(stamped, "the target's id:: is a UUID Logseq can resolve").toMatch(V4_UUID);
      expect(doc.byId["reference-authoring"].raw).toBe(expected(stamped!));
      const raws = (dto: { blocks: BlockDto[] }) => { const out: string[] = []; const walk = (b: BlockDto) => { out.push(b.raw); b.children.forEach(walk); }; dto.blocks.forEach(walk); return out; };
      const fileText = [...submittedPages(save, "Source"), ...submittedPages(save, "Reference authoring")].flatMap(raws).join("\n");
      expect(fileText, "the runtime key never reaches a file").not.toContain(runtime);
    } finally { dispose(); }
  });

  it("references the picked id-less block, not an authored block whose id equals its runtime key (GH #798)", async () => {
    // Runtime keys of loaded blocks are position-derived UUIDs: a block that
    // moved into a position inherits the key another block was once stamped with.
    const shared = "f8358fac-56bd-8bb1-ba45-bd7fd1ba2add";
    const picked: BlockDto = { id: shared, raw: "picked", collapsed: false, children: [] };
    const owner: BlockDto = { id: "e1d2c3b4-a596-8877-8665-544332211000", raw: `owner\nid:: ${shared}`, collapsed: false, children: [] };
    vi.spyOn(backend(), "search").mockResolvedValue([{ page: "Source", kind: "page", blocks: [picked], evidence: [] }]);
    vi.spyOn(backend(), "getPage").mockResolvedValue({
      id: "pages/Source.md", name: "Source", kind: "page", title: "Source", pre_block: null, blocks: [structuredClone(picked), structuredClone(owner)],
    });
    openAsLoaded(await bindTestHost());
    const save = vi.spyOn(backend(), "pageSubmit");
    loadSingle(page("((picked"));
    startEditing("reference-authoring", 8);
    const { root, dispose } = mount(() => <For each={pageByName("Reference authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>);
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "((picked", 8);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("picked"));
      accept(textarea);
      await vi.waitFor(() => expect(doc.byId["reference-authoring"].raw).toMatch(/^\(\([^)]+\)\) $/));
      const inserted = doc.byId["reference-authoring"].raw.slice(2, -3);
      expect(inserted).not.toBe(shared);
      expect(inserted).toMatch(V4_UUID);
      const blocks = submittedPages(save, "Source")[0].blocks;
      expect(blocks[0].raw).toBe(`picked\nid:: ${inserted}`);
      expect(blocks[1].raw).toBe(`owner\nid:: ${shared}`);
    } finally { dispose(); }
  });

  it("leaves the source unedited and the target conflicted when the target's ID cannot publish", async () => {
    const uuid = "f8358fac-56bd-8bb1-ba45-bd7fd1ba2add";
    vi.spyOn(backend(), "search").mockResolvedValue([{
      page: "Source", kind: "page", blocks: [{ id: uuid, raw: "test", collapsed: false, children: [] }], evidence: [],
    }]);
    vi.spyOn(backend(), "getPage").mockResolvedValue({
      id: "pages/Source.md", name: "Source", kind: "page", title: "Source", pre_block: null,
      blocks: [{ id: uuid, raw: "test", collapsed: false, children: [] }],
    });
    const host = await bindTestHost();
    openAsLoaded(host);
    // The target's file changed on disk: the host takes the stamped submit as
    // a conflict (its answer reports it) instead of publishing it.
    const submit = backend().pageSubmit.bind(backend());
    const save = vi.spyOn(backend(), "pageSubmit").mockImplementation(async (...args) => {
      if (args[2] !== "pages/Source.md") return submit(...args);
      const [, id, key] = args;
      queueMicrotask(() => host.deliver({ key, answer: { id, version: 3, took: true, outcome: { kind: "applied" } },
        notice: { conflictReported: true },
        page: { version: 3, conflict: true, risk: true, disk: { kind: "file", rev: "disk-2" }, text: { kind: "unchanged" } } }));
      return null;
    });
    loadSingle(page("((test"));
    startEditing("reference-authoring", 6);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Reference authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "((test", 6);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("test"));
      accept(textarea);
      await vi.waitFor(() => expect(isConflicted("Source")).toBe(true));
      // The id-less target was stamped with a fresh UUID, not its runtime key (GH #798).
      const stamped = /^id:: (.+)$/m.exec(submittedPages(save, "Source")[0].blocks[0].raw)?.[1];
      expect(stamped).toMatch(V4_UUID);
      // A reference reaches disk only after its target ID does (STEP3 §8, Q2).
      expect(doc.byId["reference-authoring"].raw).not.toContain(stamped!);
      expect(submittedPages(save, "Reference authoring").some((dto) => JSON.stringify(dto).includes(stamped!))).toBe(false);
      // The stamped target stays in the window for conflict resolution.
      expect(doc.byId[uuid].raw).toContain(`id:: ${stamped}`);
    } finally { dispose(); }
  });

  it("does not stamp a target ID when the editor intent expires during target lookup", async () => {
    const uuid = "f8358fac-56bd-8bb1-ba45-bd7fd1ba2add";
    let finish!: (page: PageRead) => void;
    vi.spyOn(backend(), "search").mockResolvedValue([{
      page: "Source", kind: "page", blocks: [{ id: uuid, raw: "test", collapsed: false, children: [] }], evidence: [],
    }]);
    vi.spyOn(backend(), "getPage").mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
    openAsLoaded(await bindTestHost());
    const save = vi.spyOn(backend(), "pageSubmit");
    loadSingle(page("((test"));
    startEditing("reference-authoring", 6);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Reference authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "((test", 6);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("test"));
      accept(textarea);
      await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
      inputAt(textarea, "changed", 7);
      finish({ id: "pages/Source.md", name: "Source", kind: "page", title: "Source", pre_block: null,
        blocks: [{ id: uuid, raw: "test", collapsed: false, children: [] }] });
      await vi.waitFor(() => expect(pageByName("Source")).toBeTruthy());
      expect(submittedPages(save, "Source")).toEqual([]);
      expect(doc.byId[uuid].raw).toBe("test");
    } finally { dispose(); }
  });

  it("does not let an older same-location page lookup overwrite newer results", async () => {
    const pending: { query: string; resolve: (pages: PageEntry[]) => void }[] = [];
    vi.spyOn(backend(), "quickSwitch").mockImplementation((query) =>
      new Promise<PageEntry[]>((resolve) => pending.push({ query, resolve }))
    );
    loadSingle(page("[[P]]"));
    startEditing("reference-authoring", 3);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Reference authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "[[P]]", 3);
      await vi.waitFor(() => expect(pending.map((request) => request.query)).toEqual(["P"]));

      inputAt(textarea, "[[Parity]]", 8);
      await vi.waitFor(() => expect(pending.map((request) => request.query)).toEqual(["P", "Parity"]));

      pending[1].resolve([entry("Parity New")]);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("Parity New"));
      pending[0].resolve([entry("Parity Old")]);
      await Promise.resolve();
      await Promise.resolve();
      expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("Parity New");
    } finally {
      dispose();
    }
  });

  it("labels an alias suggestion with the page it belongs to and inserts the alias (GH #558, GH #482)", async () => {
    // Core returns an authored alias as its own row, on its owner's file; the
    // page inventory says whose alias it is (an existing file would win).
    vi.spyOn(backend(), "quickSwitch").mockResolvedValue([
      { name: "Tine greet", kind: "page", date_key: null, path: "pages/Welcome to Tine.md" },
      entry("Tinsel"),
    ]);
    vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: [
      { key: "welcome to tine", name: "Welcome to Tine", is_journal: false, day: null, target: { kind: "existing", id: "pages/Welcome to Tine.md", others: [] } },
      { key: "tine greet", name: "Tine greet", is_journal: false, day: null, target: { kind: "alias", owners: ["pages/Welcome to Tine.md"] } },
      { key: "tinsel", name: "Tinsel", is_journal: false, day: null, target: { kind: "existing", id: "pages/Tinsel.md", others: [] } },
    ] });
    resetPageIndex();
    await refreshPageIndex();
    loadSingle(page("[[Tin]]"));
    startEditing("reference-authoring", 5);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Reference authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "[[Tin]]", 5);
      const rowFor = (label: string) => [...document.body.querySelectorAll(".autocomplete .ac-label")]
        .find((row) => row.textContent === label)?.parentElement;
      await vi.waitFor(() => expect(rowFor("Tine greet")).toBeTruthy());
      expect(rowFor("Tine greet")?.querySelector(".ac-sub")?.textContent).toBe("alias of Welcome to Tine");
      // An ordinary page carries no alias badge.
      expect(rowFor("Tinsel")?.querySelector(".ac-sub")).toBeNull();
      (rowFor("Tine greet") as HTMLElement).dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(textarea.value.trim()).toBe("[[Tine greet]]"));
    } finally {
      dispose();
    }
  });
});
