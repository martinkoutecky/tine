import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { For, createMemo, createSignal } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { resetStore, setRaw } from "../document";
import { doc } from "../document/model";
import { endEdit } from "../editorController";
import { Block } from "./Block";
import { RefBlocks } from "./RefBlocks";
import { FailureBoundary } from "./FailureBoundary";
import { SheetTable } from "./SheetTable";
import { setDoc } from "../document/model";
import { StickyAncestor } from "./StickyAncestor";
import * as inline from "../render/inline";
import { maybeEditorIn } from "../tests/editorDriver";

vi.mock("../render/body", () => {
  return { AstBody: (props: { raw: string }) => {
    const text = createMemo(() => {
    if (props.raw.includes("broken render")) throw new Error("injected render failure");
    return props.raw;
    });
    return <div>{text()}</div>;
  } };
});

beforeAll(initParser);
afterEach(() => { endEdit("page-navigation"); resetStore(); document.body.innerHTML = ""; });

it.each([false, true])("isolates a live block throw, keeps children and siblings, and permits repair (reactive=%s)", reactive => {
  const blocks = [
    { id: "bad", raw: reactive ? "initial" : "broken render", collapsed: false, children: [{ id: "child", raw: "healthy child", collapsed: false, children: [] }] },
    { id: "good", raw: "healthy sibling", collapsed: false, children: [] },
  ];
  loadSingle({ name: "Isolation", kind: "page", title: "Isolation", pre_block: null, blocks, format: "md" });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <FailureBoundary region="This page"><For each={["bad", "good"]}>{id => <Block id={id} />}</For></FailureBoundary>, root);
  try {
    if (reactive) setRaw("bad", "broken render", { timetracking: false });
    expect(root.textContent).toContain("healthy sibling");
    expect(root.textContent).toContain("healthy child");
    expect(root.querySelector('[data-failed-region="This page"]')).toBeNull();
    expect(root.textContent).toContain("broken render");
    const raw = root.querySelector(".block-render-raw")!;
    raw.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, cancelable: true }));
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
    const editor = maybeEditorIn(root)!;
    expect(editor.value()).toBe("broken render");
    editor.replaceText("repaired"); editor.reportInput({ bubbles: true, inputType: "insertText" });
    endEdit("blur");
    expect(doc.byId.bad.raw).toBe("repaired");
    expect(root.querySelector(".region-failure")).toBeNull();
    expect(root.textContent).toContain("repaired");
  } finally { dispose(); }
});

it.each([false, true])("isolates a DTO reference throw and keeps sibling and nested reference content (reactive=%s)", reactive => {
  const [raw, setRawText] = createSignal(reactive ? "initial" : "broken render");
  const open = vi.fn();
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <FailureBoundary region="This page"><RefBlocks blocks={[
    { id: "bad", raw: raw(), collapsed: false, children: [{ id: "child", raw: "healthy child", collapsed: false, children: [] }] },
    { id: "good", raw: "healthy sibling", collapsed: false, children: [] },
  ]} page="Isolation" onOpenBlock={open} /></FailureBoundary>, root);
  try {
    setRawText("broken render");
    expect(root.textContent).toContain("healthy sibling");
    expect(root.textContent).toContain("healthy child");
    expect(root.textContent).toContain("broken render");
    expect(root.querySelector('[data-failed-region="This page"]')).toBeNull();
    root.querySelector<HTMLElement>(".block-render-raw")!.click();
    expect(open).toHaveBeenCalledWith("bad");
  } finally { dispose(); }
});

it("isolates a query table title throw and leaves the other result live", () => {
  setDoc({ byId: { query: { id: "query", raw: "{{query [[tag]]}}", children: [], parent: null, page: "Sheet", collapsed: false } },
    pages: [{ name: "Sheet", title: "Sheet", kind: "page", roots: ["query"], preBlock: null, format: "md", readOnly: false, guide: false }], feed: ["Sheet"], loaded: true });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <FailureBoundary region="This page"><SheetTable ownerId="query" rowSource="query" groups={[{
    page: "Isolation", kind: "page", blocks: [
      { id: "bad", raw: "broken render", collapsed: false, children: [] },
      { id: "good", raw: "healthy result", collapsed: false, children: [] },
    ],
  }]} /></FailureBoundary>, root);
  try {
    expect(root.textContent).toContain("healthy result");
    expect(root.querySelector(".block-render-raw")?.textContent).toBe("broken render");
    expect(root.querySelector('[data-failed-region="This page"]')).toBeNull();
  } finally { dispose(); }
});

it("isolates the sticky ancestor's duplicate label render", () => {
  const label = vi.spyOn(inline, "InlineText").mockImplementation(() => { throw new Error("sticky label failed"); });
  const root = document.createElement("div"); document.body.append(root);
  let dispose = () => {};
  try {
    dispose = render(() => <FailureBoundary region="This page"><div>
      <StickyAncestor id="sticky" raw="repairable sticky source" /><div class="block-main">healthy row</div>
    </div></FailureBoundary>, root);
    expect(root.textContent).toContain("healthy row");
    expect(root.textContent).toContain("repairable sticky source");
    expect(root.querySelector('[data-failed-region="This page"]')).toBeNull();
  } finally { dispose(); label.mockRestore(); }
});
