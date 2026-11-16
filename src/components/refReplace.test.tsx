// D-20261019-QC(5): the block-reference right-click menu offers OG's "Replace
// with text" and "Replace with embed" (OG content.cljs block-ref-custom-context-
// menu-content, editor.cljs replace-ref-with-text! / replace-ref-with-embed!)
// on the one occurrence that was clicked, one undo step, through the ordinary
// block save.
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore, setRaw, undo } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { Block } from "./Block";
import { ContextMenu } from "./ContextMenu";
import { endEdit } from "../editorController";
import { installKeybindings } from "../keybindings";
import { closeContextMenu } from "../ui";
import { resetPaneLayoutToSingle } from "../panes";
import { toasts } from "../toasts";
import { referenceReplacementText } from "../refOccurrence";

const T = "41700000-0000-4000-8000-0000000000a1";
let targetRaw = `target text\nid:: ${T}`;

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
  targetRaw = `target text\nid:: ${T}`;
});
const tick = async () => { for (let i = 0; i < 4; i++) await new Promise(r => setTimeout(r, 20)); };

function mountHost(raw: string, format: "md" | "org" = "md") {
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(() => ({
    page: "Source", kind: "page" as const, blocks: [{ id: "target", raw: targetRaw, collapsed: false, children: [] }],
  })));
  vi.spyOn(backend(), "resolveBlock").mockImplementation(async () => ({
    page: "Source", kind: "page" as const, blocks: [{ id: "target", raw: targetRaw, collapsed: false, children: [] }],
  }));
  loadSingle({ name: "Host", title: "Host", kind: "page", format, pre_block: null,
    blocks: [{ id: "host", raw, collapsed: false, children: [] }] });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <><Block id="host" /><ContextMenu /></>, root);
  return { root, dispose };
}
const rightClick = (el: Element) =>
  el.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: 5, clientY: 5 }));
const menuItem = (label: string) => [...document.querySelectorAll<HTMLElement>(".ctx-item")].find((el) => el.textContent === label);
const warned = () => toasts().some((t) => t.kind === "warn");

describe("block reference menu: Replace with text / Replace with embed (D-20261019-QC 5)", () => {
  it("lists both rows beside Edit link and Remove reference", async () => {
    const { root, dispose } = mountHost(`see ((${T}))`);
    try {
      await tick();
      rightClick(root.querySelector(".block-ref")!);
      await tick();
      expect(menuItem("Replace with text")).toBeDefined();
      expect(menuItem("Replace with embed")).toBeDefined();
    } finally { dispose(); }
  });

  for (const format of ["md", "org"] as const) {
    it(`${format}: Replace with text puts the referenced block's text where the clicked one of two identical references stood, and Undo restores it`, async () => {
      const raw = `see ((${T})) and ((${T})) end`;
      const { root, dispose } = mountHost(raw, format);
      try {
        await tick();
        const refs = root.querySelectorAll(".block-ref");
        expect(refs).toHaveLength(2);
        rightClick(refs[1]);
        await tick();
        menuItem("Replace with text")!.click();
        await tick();
        expect(doc.byId.host.raw).toBe(`see ((${T})) and target text end`);
        undo();
        expect(doc.byId.host.raw).toBe(raw);
      } finally { dispose(); }
    });
  }

  it("Replace with text leaves out the id, collapsed and LOGBOOK of the source and keeps its visible properties", async () => {
    targetRaw = `DONE target text\nfoo:: bar\ncollapsed:: true\n:LOGBOOK:\nCLOCK: [2026-10-01 Thu 09:00:00]--[2026-10-01 Thu 09:10:00] =>  00:10:00\n:END:\nid:: ${T}`;
    const { root, dispose } = mountHost(`see ((${T})) end`);
    try {
      await tick();
      rightClick(root.querySelector(".block-ref")!);
      await tick();
      menuItem("Replace with text")!.click();
      await tick();
      expect(doc.byId.host.raw).toBe("see DONE target text\nfoo:: bar end");
    } finally { dispose(); }
  });

  it("a labelled reference is replaced whole, label included", async () => {
    const { root, dispose } = mountHost(`[my label](((${T}))) rest`);
    try {
      await tick();
      rightClick(root.querySelector(".block-ref")!);
      await tick();
      menuItem("Replace with text")!.click();
      await tick();
      expect(doc.byId.host.raw).toBe("target text rest");
    } finally { dispose(); }
  });

  it("Replace with embed turns only the clicked reference into an embed of the same block, and Undo restores it", async () => {
    const raw = `see ((${T})) and ((${T}))`;
    const { root, dispose } = mountHost(raw);
    try {
      await tick();
      rightClick(root.querySelectorAll(".block-ref")[0]);
      await tick();
      menuItem("Replace with embed")!.click();
      await tick();
      expect(doc.byId.host.raw).toBe(`see {{embed ((${T}))}} and ((${T}))`);
      undo();
      expect(doc.byId.host.raw).toBe(raw);
    } finally { dispose(); }
  });

  it("an open menu refuses, changing nothing, once the block changed after it was shown", async () => {
    const { root, dispose } = mountHost(`left ((${T})) middle ((${T})) right`);
    try {
      await tick();
      rightClick(root.querySelector(".block-content .block-ref")!);
      await tick();
      const changed = `\`lit ((${T}))\` suffix`;
      setRaw("host", changed);
      await tick();
      menuItem("Replace with embed")!.click();
      expect(doc.byId.host.raw).toBe(changed);
      expect(warned()).toBe(true);
    } finally { dispose(); }
  });

  it("an open menu's Replace with text also refuses after the block changed", async () => {
    const { root, dispose } = mountHost(`left ((${T})) middle ((${T})) right`);
    try {
      await tick();
      rightClick(root.querySelector(".block-content .block-ref")!);
      await tick();
      const changed = `left ((${T})) right`;
      setRaw("host", changed);
      await tick();
      menuItem("Replace with text")!.click();
      await tick();
      expect(doc.byId.host.raw).toBe(changed);
      expect(warned()).toBe(true);
    } finally { dispose(); }
  });
});

describe("referenceReplacementText", () => {
  it("drops hidden properties and the logbook, not a property-looking line inside a code fence", () => {
    const raw = "code\n```\nid:: not-a-property\n```\nid:: abc";
    expect(referenceReplacementText(raw, "md")).toBe("code\n```\nid:: not-a-property\n```");
  });
  it("drops the user's :block-hidden-properties too", () => {
    expect(referenceReplacementText("x\nsecret:: 1\nkeep:: 2", "md", ["secret"])).toBe("x\nkeep:: 2");
  });
  it("org: drops an all-hidden PROPERTIES drawer", () => {
    expect(referenceReplacementText("* TODO x\n:PROPERTIES:\n:ID: abc\n:END:\n", "org")).toBe("* TODO x");
  });
});
