import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { applyChangeSummary } from "../changeSummary";
import { bumpDataRev, bumpGraphEpoch } from "../graphSession";
import { resetStore } from "../document";
import { InlineText } from "../render/inline";
import { EmbedMacro } from "./EmbedMacro";
import { SidebarFavorites } from "./SidebarFavorites";
import type { ChangeSummary } from "../backendTypes";
import type { BlockDto, RefGroup } from "../types";

const id = "33333333-3333-4333-8333-333333333333";
vi.mock("../favorites", async original => {
  const real = await original<typeof import("../favorites")>();
  const { blockFavoriteNode } = await import("../favoritesLayout");
  return { ...real, favoritesLayout: () => [blockFavoriteNode("33333333-3333-4333-8333-333333333333")] };
});
// The native journey covers working-set hydration; here observe read scheduling
// and the rendered answer independently of descendant hydration.
vi.mock("./LiveRefGroup", () => ({ LiveRefGroup: (props: { blocks: BlockDto[] }) =>
  <div class="answer"><For each={props.blocks}>{block => <span>{block.raw}</span>}</For></div> }));

let rev = 0;
let dispose: (() => void) | undefined;
const tick = () => new Promise(resolve => setTimeout(resolve, 0));
function publish(fields: Partial<ChangeSummary> = {}) {
  applyChangeSummary({ rev: String(++rev), full: false, pages: [], blocks: [], linksTo: [], props: {}, inventoryChanged: false, ...fields });
  bumpDataRev(true);
}
const block = (raw: string): BlockDto => ({ id, raw, children: [], collapsed: false });
const group = (raw: string): RefGroup => ({ page: "Source", kind: "page", blocks: [block(raw)] });
beforeEach(() => { resetStore(); bumpGraphEpoch(); rev = 0; publish(); });
afterEach(() => { dispose?.(); dispose = undefined; vi.restoreAllMocks(); document.body.innerHTML = ""; });

it.each(["inline reference", "block embed", "favorite block"])("%s refreshes its block, full and gaps, without resolving after an unrelated save", async reader => {
  let text = "Original block";
  const read = vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(() => group(text)));
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => reader === "inline reference" ? <InlineText text={`((${id}))`} />
    : reader === "block embed" ? <EmbedMacro body={`embed ((${id}))`} />
    : <SidebarFavorites isActive={() => false} targetName={name => name} open={() => {}} />, root);
  await vi.waitFor(() => expect(root.textContent).toContain(text));
  read.mockClear();
  publish({ pages: ["unrelated"], blocks: ["unrelated"] });
  await tick();
  expect(read).not.toHaveBeenCalled();
  for (const fields of [{ blocks: [id] }, { full: true }, { gap: true }]) {
    text += " changed";
    if ("gap" in fields) rev += 2;
    publish(fields);
    await vi.waitFor(() => expect(root.textContent).toContain(text));
    expect(read).toHaveBeenCalledWith([id]);
    read.mockClear();
  }
});

it("a page embed refreshes its canonical page, full and gaps, without get_page after an unrelated save", async () => {
  let text = "Original page";
  const read = vi.spyOn(backend(), "getPage").mockImplementation(async name => ({ id: `pages/${name}.md`, name, kind: "page", title: name, pre_block: null, blocks: [block(text)] }));
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <EmbedMacro body="embed [[Source]]" />, root);
  await vi.waitFor(() => expect(root.textContent).toContain(text));
  read.mockClear();
  publish({ pages: ["unrelated"] }); await tick();
  expect(read).not.toHaveBeenCalled();
  for (const fields of [{ pages: ["source"] }, { full: true }, { gap: true }]) {
    text += " changed";
    if ("gap" in fields) rev += 2;
    publish(fields);
    await vi.waitFor(() => expect(root.textContent).toContain(text));
    expect(read).toHaveBeenCalledWith("Source", "page");
    read.mockClear();
  }
});

it("an open block peek reloads its subtree, full and gaps, without preview_block after an unrelated save", async () => {
  vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([group("Reference")]);
  let text = "Original peek";
  const preview = vi.spyOn(backend(), "previewBlock").mockImplementation(async () => ({ group: group(text), truncated: 0 }));
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <InlineText text={`((${id}))`} />, root);
  await vi.waitFor(() => expect(root.textContent).toContain("Reference"));
  root.querySelector(".block-ref")!.dispatchEvent(new PointerEvent("pointerenter", { pointerType: "mouse", isPrimary: true }));
  await vi.waitFor(() => expect(preview).toHaveBeenCalled(), { timeout: 2000 });
  preview.mockClear();
  publish({ blocks: ["unrelated"] }); await tick();
  expect(preview).not.toHaveBeenCalled();
  for (const fields of [{ blocks: [id] }, { full: true }, { gap: true }]) {
    text += " changed";
    if ("gap" in fields) rev += 2;
    publish(fields);
    await vi.waitFor(() => expect(preview).toHaveBeenCalled());
    preview.mockClear();
  }
});

it("a page embed answered under another name refreshes when that page changes", async () => {
  let text = "Answered page";
  const read = vi.spyOn(backend(), "getPage").mockImplementation(async () => ({ id: "pages/Target.md", name: "Target", kind: "page", title: "Target", pre_block: null, blocks: [block(text)] }));
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <EmbedMacro body="embed [[Other Spelling]]" />, root);
  await vi.waitFor(() => expect(root.textContent).toContain(text));
  await tick(); read.mockClear();
  publish({ pages: ["unrelated"] }); await tick();
  expect(read).not.toHaveBeenCalled();
  text += " changed";
  publish({ pages: ["target"] });
  await vi.waitFor(() => expect(root.textContent).toContain(text));
});
