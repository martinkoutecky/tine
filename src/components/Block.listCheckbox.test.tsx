import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore, toggleCollapse, undo } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { endEdit } from "../editorController";
import { Block } from "./Block";

beforeAll(() => initParser());
afterEach(() => { endEdit("page-navigation"); resetStore(); document.body.innerHTML = ""; });

it.each(["*", "+", "1."])("clicking %s task items toggles only their token without entering edit (GH #683)", marker => {
  const raw = `héllo\n${marker} [ ] same\n${marker} [ ] same`;
  loadSingle({ name: "Tasks", title: "Tasks", kind: "page", pre_block: null, format: "md",
    blocks: [{ id: "item", raw, collapsed: false, children: [] }] });
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <Block id="item" />, host);
  try {
    const press = () => {
      const box = host.querySelectorAll<HTMLElement>(".block-checkbox")[1];
      expect(box).toBeDefined();
      box.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
      expect(host.querySelector("textarea.block-editor")).toBeNull();
      box.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
      box.click();
    };
    press();
    expect(doc.byId.item.raw).toBe(`héllo\n${marker} [ ] same\n${marker} [x] same`);
    press();
    expect(doc.byId.item.raw).toBe(raw);
  } finally { dispose(); }
});

it.each([
  ["md", "Checklist:\n* [ ] same\n   * [ ] same"],
  ["org", "Checklist:\n- [ ] same\n   + [ ] same"],
  ["org", "Checklist:\n- [ ] same\n- [ ] same"],
] as const)("ticks the nested %s item and preserves own content on collapse (GH #691)", (format, raw) => {
  loadSingle({ name: "Tasks", title: "Tasks", kind: "page", pre_block: null, format,
    blocks: [{ id: "item", raw, collapsed: false, children: [{ id: "child", raw: "Child block", collapsed: false, children: [] }] }] });
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <Block id="item" />, host);
  try {
    const box = host.querySelectorAll<HTMLElement>(".block-checkbox")[1];
    expect(box).toBeDefined();
    box.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    box.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    box.click();
    expect(host.querySelector("textarea.block-editor")).toBeNull();
    const token = raw.lastIndexOf("[ ]");
    expect(doc.byId.item.raw).toBe(raw.slice(0, token) + "[x]" + raw.slice(token + 3));
    undo();
    expect(doc.byId.item.raw).toBe(raw);
    toggleCollapse("item");
    expect(host.querySelectorAll(".block-checkbox")).toHaveLength(2);
    expect(host.querySelector('[data-block-id="child"]')).toBeNull();
    toggleCollapse("item");
    expect(doc.byId.item.raw).toBe(raw);
  } finally { dispose(); }
});

// OG's mldoc parses these as title/paragraph text, not ListItem checkboxes.
// Keep that distinction: the renderer must not invent checkboxes from raw text.
it.each([
  ["md", "* [ ] first-line title", 0],
  ["org", "Checklist:\n- [ ] parent\n   - [ ] nested dash item", 0],
] as const)("preserves OG's literal %s checkbox-looking text", (format, raw, count) => {
  loadSingle({ name: "Tasks", title: "Tasks", kind: "page", pre_block: null, format,
    blocks: [{ id: "item", raw, collapsed: false, children: [] }] });
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <Block id="item" />, host);
  try {
    expect(host.querySelectorAll(".block-checkbox")).toHaveLength(count);
    expect(host.textContent).toContain("[ ]");
  } finally { dispose(); }
});
