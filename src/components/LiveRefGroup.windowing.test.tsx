import { afterEach, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { ensurePageLoaded, node, pageByName, resetStore } from "../document";
import { endEdit } from "../editorController";
import { closeInPageFind, openInPageFind } from "../inpageFind";
import { resetNearObserverForTests } from "../lazyObserve";
import type { BlockDto, PageRead } from "../types";
import { LiveRefGroup } from "./LiveRefGroup";
import { RefBlocks } from "./RefBlocks";

class Observer {
  static all: Observer[] = [];
  targets = new Set<Element>();
  constructor(private callback: IntersectionObserverCallback) { Observer.all.push(this); }
  observe(target: Element) { this.targets.add(target); }
  unobserve(target: Element) { this.targets.delete(target); }
  disconnect() { this.targets.clear(); }
  static near(target: Element, near: boolean) {
    for (const observer of [...Observer.all]) if (observer.targets.has(target)) observer.callback([{
      target, isIntersecting: near,
      boundingClientRect: { top: near ? 10 : 5000, bottom: near ? 210 : 5200, height: 200, width: 800 },
      rootBounds: { top: -600, bottom: 1600, height: 2200 },
    } as IntersectionObserverEntry], observer as unknown as IntersectionObserver);
  }
}
const dto = (i: number): BlockDto => ({ id: `hit-${i}`, raw: `Hit ${i}`, collapsed: false, children: [] });
const page = (i: number): PageRead => ({ id: `pages/Source ${i}.md`, name: `Source ${i}`, title: `Source ${i}`, kind: "page", format: "md", pre_block: null,
  blocks: [{ id: `parent-${i}`, raw: `Parent ${i}`, collapsed: false, children: [{ ...dto(i), children: [{ id: `child-${i}`, raw: "Child", collapsed: false, children: [] }] }] }],
} as PageRead);
function mount(count = 1) {
  vi.stubGlobal("IntersectionObserver", Observer);
  vi.spyOn(backend(), "getPage").mockImplementation(async name => page(Number(name.split(" ")[1])));
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <For each={Array.from({ length: count }, (_, i) => i)}>{i =>
    <LiveRefGroup heightKey="test-refs" page={`Source ${i}`} kind="page" blocks={[dto(i)]} surface="ref" showBreadcrumb />
  }</For>, host);
  return { host, dispose, groups: [...host.querySelectorAll<HTMLElement>(".live-ref-group")] };
}
afterEach(() => {
  closeInPageFind({ restoreFocus: false }); endEdit("page-navigation");
  resetNearObserverForTests(); resetStore(); Observer.all = [];
  document.getSelection()?.removeAllRanges();
  vi.restoreAllMocks(); vi.unstubAllGlobals(); document.body.replaceChildren();
});
it("retires visited expensive groups to their measured height instead of accumulating them", async () => {
  const { host, dispose, groups } = mount(12);
  try {
    for (let i = 0; i < groups.length; i++) {
      Observer.near(groups[i], true);
      await vi.waitFor(() => expect(groups[i].querySelector(`[data-block-id="hit-${i}"]`)).not.toBeNull());
      Observer.near(groups[i], false);
    }
    await vi.waitFor(() => expect(host.querySelectorAll(".ls-block")).toHaveLength(0));
    expect(groups[0].style.minHeight).toBe("200px");
  } finally { dispose(); }
});
it("preserves local folds and pulled-in breadcrumb context across retirement", async () => {
  const { dispose, groups: [group] } = mount();
  try {
    Observer.near(group, true);
    await vi.waitFor(() => expect(group.querySelector('[data-block-id="child-0"]')).not.toBeNull());
    (group.querySelector('[data-block-id="hit-0"] .collapse-toggle') as HTMLElement).click();
    expect(node("hit-0").collapsed).toBe(false);
    Observer.near(group, false);
    await vi.waitFor(() => expect(group.querySelector(".ls-block")).toBeNull());
    Observer.near(group, true);
    await vi.waitFor(() => expect(group.querySelector('[data-block-id="hit-0"]')).not.toBeNull());
    expect(group.querySelector('[data-block-id="child-0"]')).toBeNull();
    (group.querySelector(".ref-crumb") as HTMLElement).click();
    await vi.waitFor(() => expect(group.querySelector('[data-block-id="parent-0"]')).not.toBeNull());
    Observer.near(group, false);
    await vi.waitFor(() => expect(group.querySelector(".ls-block")).toBeNull());
    Observer.near(group, true);
    await vi.waitFor(() => expect(group.querySelector('[data-block-id="parent-0"]')).not.toBeNull());
  } finally { dispose(); }
});
it("pins the owning editor while offscreen", async () => {
  const { dispose, groups: [group] } = mount();
  try {
    Observer.near(group, true);
    await vi.waitFor(() => expect(group.querySelector('[data-block-id="child-0"]')).not.toBeNull());
    group.querySelector<HTMLElement>('[data-block-id="hit-0"] .block-content-wrapper')!
      .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
    await vi.waitFor(() => expect(group.querySelector("textarea, .cm-editor")).not.toBeNull());
    const editor = group.querySelector("textarea, .cm-editor"); expect(editor).not.toBeNull();
    Observer.near(group, false);
    expect(group.querySelector("textarea, .cm-editor")).toBe(editor);
    endEdit("page-navigation");
    await vi.waitFor(() => expect(group.querySelector(".ls-block")).toBeNull());
  } finally { dispose(); }
});
// GH #935: Find does not count reference groups, so it no longer holds them mounted.
it("retires a distant group while Find is open", async () => {
  const { dispose, groups: [group] } = mount();
  try {
    Observer.near(group, true);
    await vi.waitFor(() => expect(group.querySelector(".ls-block")).not.toBeNull());
    openInPageFind(); Observer.near(group, false);
    await vi.waitFor(() => expect(group.querySelector(".ls-block")).toBeNull());
    closeInPageFind({ restoreFocus: false });
  } finally { dispose(); }
});
it("drops an obsolete in-flight source read before document admission after a jump", async () => {
  const { dispose, groups: [group] } = mount();
  let answer!: (value: PageRead) => void;
  vi.mocked(backend().getPage).mockImplementation(() => new Promise(resolve => { answer = resolve; }));
  try {
    Observer.near(group, true);
    await vi.waitFor(() => expect(answer).toBeDefined());
    Observer.near(group, false); answer(page(0));
    await new Promise(resolve => setTimeout(resolve, 100));
    expect(pageByName("Source 0")).toBeUndefined();
  } finally { dispose(); }
});
it("does not restore visited groups, or admit unseen ones, when Find opens", async () => {
  const { dispose, groups } = mount(2);
  try {
    Observer.near(groups[0], true);
    await vi.waitFor(() => expect(groups[0].querySelector(".ls-block")).not.toBeNull());
    Observer.near(groups[0], false);
    await vi.waitFor(() => expect(groups[0].querySelector(".ls-block")).toBeNull());
    openInPageFind();
    await new Promise(resolve => setTimeout(resolve, 100));
    expect(groups[0].querySelector(".ls-block")).toBeNull();
    expect(groups[1].querySelector(".ls-block")).toBeNull();
    closeInPageFind({ restoreFocus: false });
  } finally { dispose(); }
});
it("pins a native text selection and drag until the interaction ends", async () => {
  const { dispose, groups: [group] } = mount();
  try {
    Observer.near(group, true);
    await vi.waitFor(() => expect(group.querySelector('[data-block-id="child-0"]')).not.toBeNull());
    const text = group.querySelector('[data-block-id="hit-0"] .block-content')!;
    const range = document.createRange(); range.selectNodeContents(text);
    document.getSelection()!.removeAllRanges();
    document.getSelection()!.addRange(range); document.dispatchEvent(new Event("selectionchange"));
    Observer.near(group, false);
    expect(group.querySelector(".ls-block")).not.toBeNull();
    expect(group.contains(range.startContainer)).toBe(true);
    for (let i = 1; i <= 90; i++) ensurePageLoaded(page(i));
    expect(pageByName("Source 0")).toBeDefined();
    expect(group.querySelector('[data-block-id="hit-0"] .block-content')).toBe(text);
    text.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    document.getSelection()!.removeAllRanges(); document.dispatchEvent(new Event("selectionchange"));
    expect(group.querySelector(".ls-block")).not.toBeNull();
    document.dispatchEvent(new Event("pointerup", { bubbles: true }));
    await vi.waitFor(() => expect(group.querySelector(".ls-block")).toBeNull());
  } finally { document.getSelection()?.removeAllRanges(); dispose(); }
});
it("chunks an exceptional property header so approaching it does not render every value", async () => {
  vi.stubGlobal("IntersectionObserver", Observer);
  const host = document.createElement("div"); document.body.append(host);
  const raw = Array.from({ length: 240 }, (_, i) => `key-${i}:: [[Value ${i}]]`).join("\n");
  const dispose = render(() => <RefBlocks blocks={[{ ...dto(0), raw, page_property: true }]} page="Source 0" />, host);
  try {
    expect(host.querySelectorAll(".page-property-reference-row")).toHaveLength(0);
    const chunk = [...Observer.all.flatMap(observer => [...observer.targets])].find(el => el.tagName === "SPAN")!;
    expect(chunk).toBeDefined(); Observer.near(chunk, true);
    await vi.waitFor(() => expect(host.querySelectorAll(".page-property-reference-row").length).toBeGreaterThan(0));
    expect(host.querySelectorAll(".page-property-reference-row").length).toBeLessThan(40);
    Observer.near(chunk, false);
    await vi.waitFor(() => expect(host.querySelectorAll(".page-property-reference-row")).toHaveLength(0));
  } finally { dispose(); }
});
