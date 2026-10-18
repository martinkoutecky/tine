import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore, toggleCollapse } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { rawWithCollapsed } from "../document/edits/properties";
import { splitProps, BUILTIN_HIDE } from "../editor/properties";
import { Block } from "./Block";

beforeAll(() => initParser());
afterEach(() => { resetStore(); document.body.innerHTML = ""; });

const rows = (raw: string) => raw.split("\n").filter(l => l.trim().toLowerCase().startsWith("collapsed::")).length;

// GH #721: OG (and Logseq Android) write `collapsed:: true` after the last line,
// which the parser folds into a final quote. Tine must still own, hide and
// replace that row instead of appending another on every collapse.
it.each([
  ["one row, as Logseq Android writes it", "Outside\n> Something\ncollapsed:: true"],
  ["rows already stacked by the bug", "Outside\n> Something\ncollapsed:: true\ncollapsed:: true\ncollapsed:: true"],
  ["no row yet", "Outside\n> Something"],
])("repeated collapse never stacks collapsed rows or shows them (GH #721): %s", (_name, raw) => {
  loadSingle({ name: "P", title: "P", kind: "page", format: "md", pre_block: null,
    blocks: [{ id: "p", raw, collapsed: false, children: [{ id: "c", raw: "Inside", collapsed: false, children: [] }] }] });
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <Block id="p" />, host);
  try {
    for (let i = 0; i < 6; i++) {
      toggleCollapse("p");
      const collapsed = doc.byId.p.collapsed;
      expect(rows(doc.byId.p.raw)).toBe(collapsed ? 1 : 0);
      expect(splitProps(doc.byId.p.raw, BUILTIN_HIDE).visible).toBe("Outside\n> Something");
      expect(host.textContent).not.toContain("collapsed::");
    }
    expect(doc.byId.c.raw).toBe("Inside");
  } finally { dispose(); }
});

it("an expand removes every stacked row and a collapse writes one", () => {
  const stacked = "Outside\n> Something\ncollapsed:: true\ncollapsed:: true\ncollapsed:: true";
  expect(rawWithCollapsed(stacked, false, "md")).toBe("Outside\n> Something");
  expect(rawWithCollapsed(stacked, true, "md")).toBe("Outside\n> Something\ncollapsed:: true");
});

it("quoted rows and ordinary properties stay content", () => {
  for (const raw of ["> collapsed:: true", "Outside\n> a\n> collapsed:: true", "Outside\n> a\nfoo:: bar"]) {
    expect(splitProps(raw, BUILTIN_HIDE).visible).toBe(raw);
  }
});
