import { maybeEditorIn, type EditorDriver } from "../tests/editorDriver";
import { afterEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { Block } from "../components/Block";
import { startEditing, endEdit } from "../editorController";
import { loadSingle, resetStore } from "../document/workingSet";
import { doc } from "../document/model";
import { joinProps } from "./properties";
import { existingBlockId } from "../document/edits/identity";
import { setRaw, undo, withUndoUnit } from "../document";
import { pageToDto } from "../document/convert";
import { blockRegions } from "../render/parse";
import { setGraphMeta } from "../graphSession";

afterEach(() => { endEdit("page-navigation"); resetStore(); setGraphMeta(null); document.body.innerHTML = ""; });

it.each([
  ["id:: X", ""],
  ["id:: X", "status:: active\n"],
  ["collapsed:: true", "status:: active\n"],
  ["secret:: opaque", "status:: active\n"],
])("per-keystroke property append preserves the buffer and caret (%s, %s)", (hidden, userProperty) => {
  setGraphMeta({ block_hidden_properties: ["secret"] } as Parameters<typeof setGraphMeta>[0]);
  const raw = `Title\n${hidden}\n${userProperty}body`;
  loadSingle({ name: "Typing", title: "Typing", kind: "page", format: "md", pre_block: null,
    blocks: [{ id: "target", raw, collapsed: false, children: [] }] });
  startEditing("target", 0);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Block id="target" />, root);
  try {
    const ta = maybeEditorIn(root) as EditorDriver;
    let expected = `Title\n${userProperty}body`;
    expect(ta.value()).toBe(expected);
    ta.select(expected.length, expected.length);
    for (const char of "\nowner:: Ada") {
      const caret = ta.selection()[0];
      ta.replaceText(ta.value().slice(0, caret) + char + ta.value().slice(ta.selection()[1]));
      ta.select(caret + 1, caret + 1);
      expected += char;
      ta.reportInput({ bubbles: true, inputType: "insertText", data: char });
      expect(ta.value()).toBe(expected);
      expect([ta.selection()[0], ta.selection()[1]]).toEqual([expected.length, expected.length]);
    }
    const saved = userProperty ? `Title\n${userProperty}owner:: Ada\n${hidden}\nbody`
      : `Title\nbody\nowner:: Ada\n${hidden}`;
    expect(doc.byId.target.raw).toBe(saved);
    expect(blockRegions(saved).property_regions).toHaveLength(1);
    // A user can place the caret back in the body while the saved layout differs.
    const caret = expected.indexOf("body") + 2;
    ta.replaceText(expected.slice(0, caret) + "!" + expected.slice(caret));
    ta.select(caret + 1, caret + 1);
    ta.reportInput({ bubbles: true, inputType: "insertText", data: "!" });
    expect(ta.value()).toBe(expected.replace("body", "bo!dy"));
    expect([ta.selection()[0], ta.selection()[1]]).toEqual([caret + 1, caret + 1]);
    expect(doc.byId.target.raw).toBe(saved.replace("body", "bo!dy"));
    endEdit("page-navigation");
    startEditing("target", 0);
    expect((maybeEditorIn(root) as EditorDriver).value())
      .toBe(userProperty ? `Title\n${userProperty}owner:: Ada\nbo!dy` : "Title\nbo!dy\nowner:: Ada");
  } finally { dispose(); }
});

it.each([
  ["Title\nid:: X\nbody", "Title\nbody\nowner:: Ada"],
  ["Title\nowner:: Ada\nid:: X\nbody", "Title\nowner:: Ada Lovelace\nnote\nbody"],
  ["Title\nowner:: Ada\nbody\nid:: X", "Title X\nowner:: Ada\nbody"],
])("editor input keeps all properties in OG's first group (%s)", (raw, text) => {
  loadSingle({ name: "Group", title: "Group", kind: "page", format: "md", pre_block: null,
    blocks: [{ id: "target", raw, collapsed: false, children: [] }] });
  startEditing("target", 0);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Block id="target" />, root);
  try {
    const ta = maybeEditorIn(root) as EditorDriver;
    ta.replaceText(text);
    ta.reportInput({ bubbles: true, inputType: "insertFromPaste" });
    const regions = blockRegions(doc.byId.target.raw);
    expect(regions.property_regions).toHaveLength(1);
    expect(regions.properties.filter(p => p.region === 0).map(p => p.key).sort()).toEqual(["id", "owner"]);
    expect(ta.value()).toBe(text);
  } finally { dispose(); }
});

it("explicitly typed id text takes precedence in the window editor, as in OG", () => {
  const raw = "Title\nid:: original";
  loadSingle({ name: "Identity", title: "Identity", kind: "page", format: "md", pre_block: null,
    blocks: [{ id: "target", raw, collapsed: false, children: [] }] });
  startEditing("target", 0);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Block id="target" />, root);
  try {
    const ta = maybeEditorIn(root) as EditorDriver;
    ta.replaceText("Title\nid:: supplied");
    ta.reportInput({ bubbles: true, inputType: "insertFromPaste" });
    expect(blockRegions(doc.byId.target.raw).properties.filter(p => p.key === "id").map(p => p.value)).toEqual(["supplied"]);
  } finally { dispose(); }
});

it("clearing the first editor keeps its flashcard metadata on an outline block", () => {
  const raw = "Question\ncard-repeats:: 2";
  loadSingle({ name: "Card", title: "Card", kind: "page", format: "md", pre_block: null,
    blocks: [{ id: "target", raw, collapsed: false, children: [] }] });
  startEditing("target", 0);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Block id="target" />, root);
  try {
    const ta = maybeEditorIn(root) as EditorDriver;
    expect(ta.value()).toBe("Question");
    ta.replaceText("");
    ta.reportInput({ bubbles: true, inputType: "deleteContentBackward" });
    expect(ta.value()).toBe("");
    endEdit("page-navigation");
    expect(root.querySelector(".ls-block")).not.toBeNull();
    const dto = pageToDto("Card")!;
    expect(dto.pre_block).toBeNull();
    expect(dto.blocks.map(block => block.raw)).toEqual(["\ncard-repeats:: 2"]);
  } finally { dispose(); }
});

it("Org typing reattaches the ID to the accepted mixed drawer after CLOSED", () => {
  const raw = "DONE Résumé\nCLOSED: [2026-09-30 Wed 09:00]\n:PROPERTIES:\n:owner: Jane\n:id: own-id\n:END:\nbody\n:PROPERTIES:\n:id: body-id\n:END:";
  loadSingle({ name: "Reattach", title: "Reattach", kind: "page", format: "org", pre_block: null,
    blocks: [{ id: "target", raw, collapsed: false, children: [] }] });
  startEditing("target", 0);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Block id="target" />, root);
  try {
    const ta = maybeEditorIn(root) as EditorDriver;
    ta.replaceText(ta.value() + "!");
    ta.reportInput({ bubbles: true, inputType: "insertText", data: "!" });
    expect(doc.byId.target.raw).toBe(raw + "!");
    expect(ta.value()).not.toContain(":id: own-id");
    expect(ta.value()).toContain(":id: body-id");
    expect(root.querySelector(".ls-block")?.getAttribute("data-block-ref")).toBe("own-id");
    expect(existingBlockId(doc.byId.target.raw, "org")).toBe("own-id");
    withUndoUnit("outside-edit", ["Reattach"], () => setRaw("target", "Other\n:PROPERTIES:\n:owner: Alex\n:id: external-id\n:END:", { timetracking: false }));
    expect(ta.value()).toBe("Other\n:PROPERTIES:\n:owner: Alex\n:END:");
    expect(root.querySelector(".ls-block")?.getAttribute("data-block-ref")).toBe("external-id");
    undo();
    expect(doc.byId.target.raw).toBe(raw + "!");
    expect(root.querySelector(".ls-block")?.getAttribute("data-block-ref")).toBe("own-id");
  } finally { dispose(); }
});

it("Org mixed-drawer typing on a 2,000-block page adds no ownership parse", () => {
  const raw = "DONE Large\nCLOSED: [2026-09-30 Wed 09:00]\n:PROPERTIES:\n:owner: Jane\n:id: large-id\n:END:";
  loadSingle({ name: "Large", title: "Large", kind: "page", format: "org", pre_block: null,
    blocks: [{ id: "target", raw, collapsed: false, children: [] }, ...Array.from({ length: 1999 }, (_, i) =>
      ({ id: `large-${i}`, raw: `prose ${i}`, collapsed: false, children: [] }))] });
  startEditing("target", 0);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Block id="target" />, root);
  const w = window as unknown as { __tineBench?: boolean; __tineParseStats?: { calls: number; hits: number; misses: number } };
  w.__tineBench = true; w.__tineParseStats = { calls: 0, hits: 0, misses: 0 };
  try {
    const ta = maybeEditorIn(root) as EditorDriver;
    for (let i = 0; i < 200; i++) {
      ta.replaceText(`x${ta.value()}`);
      ta.select(1, 1);
      ta.reportInput({ bubbles: true, inputType: "insertText", data: "x" });
    }
    console.info(`OG-R4C Org mixed drawer at 2k: ${JSON.stringify(w.__tineParseStats)}`);
    expect(w.__tineParseStats.misses).toBe(200);
    expect(doc.byId.target.raw).toBe("x".repeat(200) + raw);
  } finally { dispose(); w.__tineBench = false; }
});

it("Org reattachment leaves a leading literal wrapper intact", () => {
  const visible = "#+BEGIN_SRC text\n:owner: literal\n#+END_SRC";
  expect(joinProps(visible, ":id: own-id", "org")).toBe(visible + "\n:PROPERTIES:\n:id: own-id\n:END:");
});

it("Org reattachment preserves CRLF transport and an emptied own drawer", () => {
  const visible = "Résumé\r\nCLOSED: [2026-09-30 Wed 09:00]\r\n:PROPERTIES:\r\n:owner: Jane\r\n:END:\r\nbody";
  expect(joinProps(visible, ":id: own-id\r", "org")).toBe(visible.replace(":END:", ":id: own-id\r\n:END:"));
  expect(joinProps("Title\n:PROPERTIES:\n:END:", ":id: own-id", "org"))
    .toBe("Title\n:PROPERTIES:\n:id: own-id\n:END:");
});

// REG-HIDDEN-PROPS-STAY-IN-THE-GROUP: typing in a Markdown block whose hidden id sits in its
// property group keeps one group. Below the body it was a second group, which OG ignores.
it("Markdown typing keeps a hidden id in the block's own property group", () => {
  const raw = "Title\nid:: 650e8400-e29b-41d4-a716-446655440000\nowner:: Ada\nbody line";
  loadSingle({ name: "Group", title: "Group", kind: "page", format: "md", pre_block: null,
    blocks: [{ id: "target", raw, collapsed: false, children: [] }] });
  startEditing("target", 0);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Block id="target" />, root);
  try {
    const ta = maybeEditorIn(root) as EditorDriver;
    expect(ta.value()).toBe("Title\nowner:: Ada\nbody line");
    ta.replaceText(ta.value() + "!");
    ta.reportInput({ bubbles: true, inputType: "insertText", data: "!" });
    // The id stays exactly where it was in the group, never below the body.
    expect(doc.byId.target.raw).toBe(raw + "!");
    // The same holds before a final quote, which otherwise takes the hidden rows above it.
    expect(joinProps("Title\nowner:: Ada\nbody\n> quote", "id:: x", "md")).toBe("Title\nowner:: Ada\nid:: x\nbody\n> quote");
    expect(joinProps("Title\n> quote", "id:: x", "md")).toBe("Title\nid:: x\n> quote");
  } finally { dispose(); }
});
