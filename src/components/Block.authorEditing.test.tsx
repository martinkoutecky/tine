import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { For } from "solid-js";
import { Block } from "./Block";
import { ContextMenu } from "./ContextMenu";
import { blockRegions, initParser } from "../render/parse";
import { facetsOf } from "../render/facets";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { pageByName, resetStore, undo } from "../document";
import { endEdit, startEditing } from "../editorController";
import { closeContextMenu } from "../ui";
import { bindTestHost } from "../document/host/wiring.test.support";
import { editorIn } from "../tests/editorDriver";
import { backend } from "../backend";

let dispose: (() => void) | undefined;
beforeAll(() => initParser());
beforeEach(async () => {
  await bindTestHost();
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(() => null));
});
afterEach(() => {
  endEdit("page-navigation"); dispose?.(); closeContextMenu(); resetStore();
  dispose = undefined; vi.restoreAllMocks();
  document.body.innerHTML = "";
});
function mount(raw: string, format: "md" | "org" = "md") {
  const properties = blockRegions(raw, format).properties.filter(p => p.primary).map((p): [string, string] => [p.key, p.value]);
  loadSingle({ name: "Authors", title: "Authors", kind: "page", format, pre_block: null,
    blocks: [{ id: "p", raw, properties, collapsed: false, children: [] }] });
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <><For each={pageByName("Authors")?.roots ?? []}>{id => <Block id={id} />}</For><ContextMenu /></>, root);
  return root;
}

it.each(["md", "org"] as const)("ordinary %s authors stay marked, hidden and byte-exact through editing (GH #894)", format => {
  const meta = format === "md" ? "tine.author:: Ada Test  \t" : ":PROPERTIES:\n:tine.author: Ada Test  \t\n:END:";
  const root = mount(`Words\n${meta}`, format);
  expect(root.querySelector(".author-chip")?.textContent).toBe("Ada Test");
  startEditing("p", 0);
  const editor = editorIn(root);
  expect(editor.value()).toBe("Words");
  expect(root.querySelector(".author-chip")?.textContent).toBe("Ada Test");
  expect(root.querySelector(".ls-block.authored")).not.toBeNull();
  editor.input("Revised words"); endEdit("page-navigation");
  expect(doc.byId.p.raw).toBe(`Revised words\n${meta}`);
});

it("the author's chip menu removes attribution, keeps a live draft and supports undo (GH #894)", async () => {
  const root = mount("Words\ntine.author:: Ada Test  \t\nstatus:: draft");
  startEditing("p", 0);
  editorIn(root).input("Revised words\nstatus:: draft");
  const chip = root.querySelector(".author-chip");
  expect(chip).not.toBeNull();
  // Retain editor focus until click: native blur would otherwise replace this chip.
  const down = new MouseEvent("mousedown", { button: 0, bubbles: true, cancelable: true });
  chip!.dispatchEvent(down);
  expect(down.defaultPrevented).toBe(true);
  (chip as HTMLElement).click();
  const remove = () => [...root.querySelectorAll<HTMLButtonElement>(".ctx-item")].find(b => b.textContent?.trim() === "Remove author");
  await vi.waitFor(() => expect(remove()).toBeDefined());
  remove()!.click(); endEdit("page-navigation");
  expect(doc.byId.p.raw).toBe("Revised words\nstatus:: draft");
  expect(root.querySelector(".author-chip, .ls-block.authored")).toBeNull();
  undo();
  expect(doc.byId.p.raw).toContain("tine.author:: Ada Test  \t");
  expect(doc.byId.p.raw).toContain("Revised words");
});

it.each(["> some quoted text", "> first\n> second", "> [!TIP]\n> helpful", "Title\n> quoted body"])("a trailing author never becomes quote text: %j (GH #895)", body => {
  const raw = `${body}\ntine.author:: Ada Test  \t`;
  const root = mount(raw);
  expect(root.querySelector(".md-quote, .callout")?.textContent).not.toContain("tine.author");
  expect(root.querySelector(".author-chip")?.textContent).toBe("Ada Test");
  expect(doc.byId.p.raw).toBe(raw); // Reading does not migrate the file.
  startEditing("p", 0);
  expect(editorIn(root).value()).toBe(body);
  editorIn(root).input(body.replace("quoted", "revised")); endEdit("page-navigation");
  expect(doc.byId.p.raw).toContain("tine.author:: Ada Test  \t");
  expect(facetsOf(doc.byId.p.raw, "md").properties).toContainEqual(["tine.author", "Ada Test"]);
  expect(root.querySelector(".md-quote, .callout")?.textContent).not.toContain("tine.author");
});

it.each(["> tine.author:: quoted example", "> quote\n> tine.author:: quoted example", "```\ntine.author:: example\n```", "> quote\nuser.author:: example"])("authorship lookalikes stay editable content: %j", raw => {
  const root = mount(raw); startEditing("p", 0);
  const editor = editorIn(root);
  // Block entry opens source in both editors; property-looking code stays text.
  expect(editor.value()).toBe(raw);
  const at = raw.indexOf("example");
  editor.select(at, at + "example".length);
  editor.type("revised example");
  endEdit("page-navigation");
  expect(doc.byId.p.raw).toBe(raw.replace("example", "revised example"));
});
