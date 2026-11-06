import { editorIn } from "../tests/editorDriver";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { editingId, startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { noteLayoutRtl, setNoteLayoutRtl } from "../noteLayoutSetting";
import { Block } from "./Block";

const FA = "یادداشت";
let dispose = () => {};
beforeAll(() => initParser());
afterEach(() => { dispose(); dispose = () => {}; setNoteLayoutRtl(false); resetStore(); document.body.innerHTML = ""; });

function mount(raws: string[]) {
  loadSingle({ name: "P", kind: "page", title: "P", pre_block: null, blocks: raws.map((raw, i) => ({ id: `b${i}`, raw, collapsed: false, children: [] })) });
  const root = document.body.appendChild(document.createElement("div"));
  dispose = render(() => <For each={pageByName("P")?.roots ?? []}>{id => <Block id={id} />}</For>, root);
  return root;
}
const roots = () => pageByName("P")!.roots;

describe("a right-to-left block in the outline (GH #823)", () => {
  it("has the same direction rendered and in the editor, and Latin source does not flip it", () => {
    const root = mount([`TODO ${FA}`, "plain words"]);
    const rendered = root.querySelectorAll(".block-content");
    expect(rendered[0].getAttribute("dir")).toBe("rtl");
    expect(rendered[1].hasAttribute("dir")).toBe(false);
    startEditing(roots()[0], 0);
    expect(root.querySelector(".editor-wrap")!.getAttribute("dir")).toBe("rtl");
  });

  it("ArrowRight leaves backward and ArrowLeft stays inside at offset 0", async () => {
    const root = mount(["before", FA, "after"]);
    startEditing(roots()[1], 0);
    // CM reads the inherited CSS direction during its first layout measure.
    // Give it that frame before sending a user's arrow key.
    await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    editorIn(root).keydown({ key: "ArrowLeft", bubbles: true, cancelable: true });
    expect(editingId()).toBe(roots()[1]);
    // The first key moves CM's caret into the text; test the other edge key
    // from offset zero again (synthetic textarea keys do not move its caret).
    editorIn(root).select(0);
    editorIn(root).keydown({ key: "ArrowRight", bubbles: true, cancelable: true });
    expect(editingId()).toBe(roots()[0]);
  });

  it("ArrowLeft leaves forward at the end", async () => {
    const root = mount(["before", FA, "after"]);
    startEditing(roots()[1], FA.length);
    await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    editorIn(root).keydown({ key: "ArrowLeft", bubbles: true, cancelable: true });
    expect(editingId()).toBe(roots()[2]);
  });

  it("the note layout setting mirrors rows and writes nothing into the text", () => {
    const root = mount(["plain words", FA]);
    const before = pageByName("P")!.roots.length;
    expect(root.querySelector(".ls-block")!.hasAttribute("dir")).toBe(false);
    setNoteLayoutRtl(true);
    expect(noteLayoutRtl()).toBe(true);
    expect(root.querySelector(".ls-block")!.getAttribute("dir")).toBe("rtl");
    expect(root.querySelectorAll(".block-content")[0].getAttribute("dir")).toBe("ltr");
    expect(pageByName("P")!.roots.length).toBe(before);
  });
});
