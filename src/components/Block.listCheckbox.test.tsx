import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
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
