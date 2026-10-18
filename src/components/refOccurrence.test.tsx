// GH #417 (with #418's toolbar remainder): a block reference, block embed or
// page embed offers Edit link, Open source and Remove for that ONE occurrence.
// Remove edits only the containing block's text (never the referenced block or
// page), one undo step; Edit link opens the containing block's editor with the
// occurrence selected; Open source goes to the referenced block or page.
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore, setRaw, undo } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { Block } from "./Block";
import { ContextMenu } from "./ContextMenu";
import { endEdit, editingId } from "../editorController";
import { installKeybindings } from "../keybindings";
import { closeContextMenu } from "../ui";
import { paneRouter, resetPaneLayoutToSingle } from "../panes";
import { PeekContext } from "../render/PeekPopup";
import { EmbedMacro } from "./Macro";
import { removeOccurrence } from "../refOccurrence";
import { toasts } from "../toasts";
import { bumpDataRev } from "../graphSession";

const T = "41700000-0000-4000-8000-000000000001";
const TARGET_RAW = `target text\nid:: ${T}`;

let disposeKeys: () => void;
beforeAll(async () => { await initParser(); disposeKeys = installKeybindings(); });
afterAll(() => disposeKeys());
afterEach(() => {
  endEdit("page-navigation");
  closeContextMenu();
  resetStore();
  resetPaneLayoutToSingle();
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});
const tick = async () => { for (let i = 0; i < 4; i++) await new Promise(r => setTimeout(r, 20)); };

function source(format: "md" | "org" = "md") {
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(() => ({
    page: "Source", kind: "page" as const, blocks: [{ id: "target", raw: TARGET_RAW, collapsed: false, children: [] }],
  })));
  vi.spyOn(backend(), "getPage").mockImplementation(async name => ({
    id: `pages/${name}`, name, title: name, kind: "page", format, pre_block: null,
    blocks: [{ id: "page-row", raw: "page row text", collapsed: false, children: [] }],
  }) as never);
}

function mountHost(raw: string, format: "md" | "org" = "md") {
  source(format);
  loadSingle({ name: "Host", title: "Host", kind: "page", format, pre_block: null,
    blocks: [{ id: "host", raw, collapsed: false, children: [] }] });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <><Block id="host" /><ContextMenu /></>, root);
  return { root, dispose };
}

const actions = (root: HTMLElement, cls: string) => [...root.querySelectorAll<HTMLButtonElement>(`.embed-actions .${cls}`)];

describe("embed actions (GH #417/#418)", () => {
  for (const format of ["md", "org"] as const) {
    it(`${format}: Remove takes out only the clicked one of two identical embeds, and Undo restores it`, async () => {
      const raw = `A {{embed ((${T}))}} and {{embed ((${T}))}} end`;
      const { root, dispose } = mountHost(raw, format);
      try {
        await tick();
        expect(root.textContent).toContain("target text");
        const remove = actions(root, "embed-action-remove");
        expect(remove).toHaveLength(2);
        remove[1].click();
        await tick();
        expect(doc.byId.host.raw).toBe(`A {{embed ((${T}))}} and end`);
        expect(root.querySelectorAll(".embed-block")).toHaveLength(1);
        undo();
        expect(doc.byId.host.raw).toBe(raw);
      } finally { dispose(); }
    });
  }

  it("a whole-block embed keeps its (now empty) block when removed; the source block is not touched", async () => {
    const { root, dispose } = mountHost(`{{embed ((${T}))}}`);
    try {
      await tick();
      actions(root, "embed-action-remove")[0].click();
      await tick();
      expect(doc.byId.host.raw).toBe("");
      expect(doc.byId.target).toBeUndefined(); // the source page was never loaded, let alone edited
    } finally { dispose(); }
  });

  it("Edit link opens the containing block's editor with the embed selected", async () => {
    const raw = `{{embed ((${T}))}}`;
    const { root, dispose } = mountHost(raw);
    try {
      await tick();
      actions(root, "embed-action-edit")[0].click();
      await tick();
      expect(editingId()).toBe("host");
      const editor = root.querySelector<HTMLTextAreaElement>('[data-block-id="host"] textarea.block-editor')!;
      expect(editor.value.slice(editor.selectionStart, editor.selectionEnd)).toBe(raw);
    } finally { dispose(); }
  });

  it("Open source goes to the embedded block, and for a page embed to the page", async () => {
    resetPaneLayoutToSingle();
    const { root, dispose } = mountHost(`{{embed ((${T}))}} {{embed [[Elsewhere]]}}`);
    try {
      await tick();
      const open = actions(root, "embed-action-source");
      expect(open).toHaveLength(2);
      open[0].click();
      // Opens in place at the block (the route keeps the page; the block is focused).
      expect(paneRouter("main").route()).toMatchObject({ kind: "page", name: "Source" });
      open[1].click();
      expect(paneRouter("main").route()).toMatchObject({ kind: "page", name: "Elsewhere" });
      expect(doc.byId.host.raw).toBe(`{{embed ((${T}))}} {{embed [[Elsewhere]]}}`);
    } finally { dispose(); }
  });

  it("Open source on an embed shown inside a block reference goes to the embedded block, not the reference's", async () => {
    const U = "41700000-0000-4000-8000-000000000002";
    vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map((id) => id === T
      ? { page: "Holder", kind: "page" as const, blocks: [{ id: T, raw: `{{embed ((${U}))}}\nid:: ${T}`, collapsed: false, children: [] }] }
      : { page: "Source", kind: "page" as const, blocks: [{ id: U, raw: `target text\nid:: ${U}`, collapsed: false, children: [] }] }));
    loadSingle({ name: "Host", title: "Host", kind: "page", pre_block: null,
      blocks: [{ id: "host", raw: `see ((${T}))`, collapsed: false, children: [] }] });
    bumpDataRev(); // T resolved to a different block in earlier cases
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <Block id="host" />, root);
    try {
      await tick(); await tick();
      const open = root.querySelector<HTMLButtonElement>(".block-ref .embed-action-source");
      expect(open).not.toBeNull();
      open!.click();
      expect(paneRouter("main").route()).toMatchObject({ kind: "page", name: "Source" });
    } finally { dispose(); }
  });

  it("a missing embed can still be edited and removed; a preview offers no actions", async () => {
    const missing = "41700000-0000-4000-8000-0000000000ff";
    vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([] as never);
    loadSingle({ name: "Host", title: "Host", kind: "page", pre_block: null,
      blocks: [{ id: "host", raw: `x {{embed ((${missing}))}}`, collapsed: false, children: [] }] });
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <><Block id="host" />
      <PeekContext.Provider value={true}><EmbedMacro body={`embed ((${missing}))`} blockId="host" span={[4, 54]} /></PeekContext.Provider></>, root);
    try {
      await tick();
      expect(actions(root, "embed-action-source")).toHaveLength(0);
      expect(actions(root, "embed-action-remove")).toHaveLength(1); // the preview copy has none
      actions(root, "embed-action-remove")[0].click();
      expect(doc.byId.host.raw).toBe("x");
    } finally { dispose(); }
  });
});

describe("block reference actions (GH #417)", () => {
  const rightClick = (el: Element) =>
    el.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: 5, clientY: 5 }));
  const menuItem = (label: string) => [...document.querySelectorAll<HTMLElement>(".ctx-item")].find((el) => el.textContent === label);

  it("Remove reference in the right-click menu removes only that occurrence", async () => {
    const raw = `see ((${T})) and ((${T}))`;
    const { root, dispose } = mountHost(raw);
    try {
      await tick();
      const refs = root.querySelectorAll(".block-ref");
      expect(refs).toHaveLength(2);
      rightClick(refs[1]);
      await tick();
      menuItem("Remove reference")!.click();
      expect(doc.byId.host.raw).toBe(`see ((${T})) and`);
      undo();
      expect(doc.byId.host.raw).toBe(raw);
    } finally { dispose(); }
  });

  it("a labelled reference is removed whole, label included", async () => {
    const { root, dispose } = mountHost(`[my label](((${T}))) rest`);
    try {
      await tick();
      rightClick(root.querySelector(".block-ref")!);
      await tick();
      menuItem("Remove reference")!.click();
      expect(doc.byId.host.raw).toBe("rest");
    } finally { dispose(); }
  });

  it("Edit link in the menu opens the block's editor with the reference selected", async () => {
    const { root, dispose } = mountHost(`see ((${T})) here`);
    try {
      await tick();
      rightClick(root.querySelector(".block-ref")!);
      await tick();
      menuItem("Edit link")!.click();
      await tick();
      const editor = root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
      expect(editor.value.slice(editor.selectionStart, editor.selectionEnd)).toBe(`((${T}))`);
    } finally { dispose(); }
  });

  it("the hover preview's title row offers Edit link and Remove", async () => {
    vi.spyOn(backend(), "previewBlock").mockResolvedValue({
      group: { page: "Source", kind: "page", blocks: [{ id: "target", raw: TARGET_RAW, collapsed: false, children: [] }] },
      truncated: 0,
    } as never);
    const { root, dispose } = mountHost(`see ((${T}))`);
    try {
      await tick();
      const ref = root.querySelector(".block-ref")!;
      ref.dispatchEvent(new PointerEvent("pointerenter", { bubbles: false, pointerType: "mouse", isPrimary: true }));
      await new Promise(r => setTimeout(r, 450));
      await tick();
      const remove = document.querySelector<HTMLButtonElement>(".peek-popup .peek-ref-remove");
      expect(document.querySelector(".peek-popup .peek-ref-edit")).not.toBeNull();
      remove!.click();
      expect(doc.byId.host.raw).toBe("see");
      expect(document.querySelector(".peek-popup")).toBeNull();
    } finally { dispose(); }
  });
});

describe("occurrence guard", () => {
  const rightClick = (el: Element) =>
    el.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: 5, clientY: 5 }));
  const menuItem = (label: string) => [...document.querySelectorAll<HTMLElement>(".ctx-item")].find((el) => el.textContent === label)!;
  const warned = () => toasts().some((t) => t.kind === "warn" && t.message.includes("changed after it was shown"));

  it("refuses, changing nothing, when the block no longer holds the shown reference there", async () => {
    loadSingle({ name: "Host", title: "Host", kind: "page", pre_block: null,
      blocks: [{ id: "host", raw: `see ((${T}))`, collapsed: false, children: [] }] });
    // The span the render saw ("- see ((…))" bytes 6..46), then an external edit moved the text.
    const shown = doc.byId.host.raw;
    setRaw("host", `prefix text, see ((${T}))`);
    const before = doc.byId.host.raw;
    expect(removeOccurrence({ host: "host", span: [6, 46], kind: "ref", target: T, raw: shown })).toBe(false);
    expect(doc.byId.host.raw).toBe(before);
    expect(warned()).toBe(true);
  });

  // Review r1 (8a5539b18): a menu left open while the block changed must not act
  // on whatever now sits at the old span.
  it("an open menu refuses after the reference was replaced by a code literal at the same place", async () => {
    const { root, dispose } = mountHost(`safe ((${T})) suffix`);
    try {
      await tick();
      rightClick(root.querySelector(".block-content .block-ref")!);
      await tick();
      const changed = `\`lit ((${T}))\` suffix`;
      setRaw("host", changed);
      await tick();
      menuItem("Remove reference").click();
      expect(doc.byId.host.raw).toBe(changed);
      expect(warned()).toBe(true);
    } finally { dispose(); }
  });

  it("an open menu refuses after its reference was deleted and an identical one moved into its place", async () => {
    const { root, dispose } = mountHost(`left ((${T})) middle ((${T})) right`);
    try {
      await tick();
      rightClick(root.querySelector(".block-content .block-ref")!);
      await tick();
      const changed = `left ((${T})) right`;
      setRaw("host", changed);
      await tick();
      menuItem("Remove reference").click();
      expect(doc.byId.host.raw).toBe(changed);
    } finally { dispose(); }
  });

  // Review r2 (677b47917): the toolbar must act on the host text it was SHOWN
  // with. Focus a button, hold Space, a sync replaces the embed's target, release
  // Space: the press must not land on the replacement embed.
  for (const action of ["remove", "edit"] as const) {
    it(`a focused embed ${action} button cannot act after a sync replaced the embed's target`, async () => {
      const U = "41700000-0000-4000-8000-000000000002";
      const { root, dispose } = mountHost(`{{embed ((${T}))}}`);
      try {
        await tick();
        const button = root.querySelector<HTMLButtonElement>(`.embed-action-${action}`)!;
        button.focus();
        expect(document.activeElement).toBe(button);
        const changed = `{{embed ((${U}))}}`;
        setRaw("host", changed);
        await tick();
        // The Space release activates whatever still holds the focus.
        if (document.activeElement instanceof HTMLButtonElement) document.activeElement.click();
        expect(doc.byId.host.raw).toBe(changed);
        expect(editingId()).toBeNull();
        // The redrawn toolbar acts on what it now shows.
        root.querySelector<HTMLButtonElement>(`.embed-action-${action}`)!.click();
        await tick();
        if (action === "remove") expect(doc.byId.host.raw).toBe("");
        else expect(editingId()).toBe("host");
      } finally { dispose(); }
    });
  }

  it("an open menu's Edit link refuses after the block changed, opening no editor", async () => {
    const { root, dispose } = mountHost(`left ((${T})) middle ((${T})) right`);
    try {
      await tick();
      rightClick(root.querySelector(".block-content .block-ref")!);
      await tick();
      setRaw("host", `left ((${T})) right`);
      await tick();
      menuItem("Edit link").click();
      await tick();
      expect(editingId()).toBeNull();
      expect(warned()).toBe(true);
    } finally { dispose(); }
  });
});
