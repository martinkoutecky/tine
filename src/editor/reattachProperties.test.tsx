import { afterEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { Block } from "../components/Block";
import { startEditing, endEdit } from "../editorController";
import { loadSingle, resetStore } from "../document/workingSet";
import { doc } from "../document/model";
import { joinProps } from "./properties";
import { existingBlockId } from "../document/edits/identity";
import { setRaw, undo, withUndoUnit } from "../document";

afterEach(() => { endEdit("page-navigation"); resetStore(); document.body.innerHTML = ""; });

it("Org typing reattaches the ID to the accepted mixed drawer after CLOSED", () => {
  const raw = "DONE Résumé\nCLOSED: [2026-09-30 Wed 09:00]\n:PROPERTIES:\n:owner: Jane\n:id: own-id\n:END:\nbody\n:PROPERTIES:\n:id: body-id\n:END:";
  loadSingle({ name: "Reattach", title: "Reattach", kind: "page", format: "org", pre_block: null,
    blocks: [{ id: "target", raw, collapsed: false, children: [] }] });
  startEditing("target", 0);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Block id="target" />, root);
  try {
    const ta = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
    ta.value += "!";
    ta.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "!" }));
    expect(doc.byId.target.raw).toBe(raw + "!");
    expect(ta.value).not.toContain(":id: own-id");
    expect(ta.value).toContain(":id: body-id");
    expect(root.querySelector(".ls-block")?.getAttribute("data-block-ref")).toBe("own-id");
    expect(existingBlockId(doc.byId.target.raw, "org")).toBe("own-id");
    withUndoUnit("outside-edit", ["Reattach"], () => setRaw("target", "Other\n:PROPERTIES:\n:owner: Alex\n:id: external-id\n:END:", { timetracking: false }));
    expect(ta.value).toBe("Other\n:PROPERTIES:\n:owner: Alex\n:END:");
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
    const ta = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
    for (let i = 0; i < 200; i++) {
      ta.value = `x${ta.value}`;
      ta.setSelectionRange(1, 1);
      ta.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "x" }));
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
