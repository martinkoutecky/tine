import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { loadFeed } from "../document/workingSet";
import { resetStore, setCollapsed, setRaw } from "../document";
import { initParser } from "../render/parse";
import { changeStickyParents } from "../stickyParents";
import { Block } from "./Block";
import type { BlockDto, PageDto } from "../types";
import * as realm from "../windowRealm";

beforeAll(() => initParser());
afterEach(() => { resetStore(); changeStickyParents(true); vi.restoreAllMocks(); document.body.replaceChildren(); });
const leaf = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({ id, raw, collapsed: false, children });

for (const shell of ["main-content", "right-sidebar", "main-content pane-main-content"]) {
  for (const format of ["md", "org"] as const) {
    it(`${shell}, ${format}: pins the page-embed bullet above embedded parents without another live embed (GH #898)`, async () => {
      const source: PageDto & { id: string } = { id: `pages/Source.${format}`, name: "Source", title: "Source", kind: "page", format, pre_block: null,
        blocks: [leaf("source", "Source **parent**", [leaf("kid", "Embedded child")])] };
      vi.spyOn(backend(), "getPage").mockResolvedValue(source);
      loadFeed([{ name: "Host", title: "Host", kind: "page", format, pre_block: null,
        blocks: [leaf("outer", "Outer", [leaf("host", "{{embed [[Source]]}}")])] }, source]);
      const root = document.createElement("div"); root.className = shell; document.body.append(root);
      const dispose = render(() => <Block id="outer" />, root);
      try {
        await vi.waitFor(() => expect(root.querySelector('.embed-block [data-block-id="kid"]')).not.toBeNull());
        const pin = root.querySelector<HTMLElement>('[data-sticky-block="host"]');
        expect(pin, "embed's parent bullet must remain available while reading its rows").not.toBeNull();
        expect(pin!.textContent).toContain("Source");
        expect(pin!.textContent).not.toContain("{{");
        expect(pin!.querySelector(".embed-block")).toBeNull();
        expect(root.querySelectorAll(".embed-block")).toHaveLength(1);
        expect(pin!.style.top).toContain("30px");
        expect(root.querySelector<HTMLElement>('[data-sticky-block="source"]')!.style.top).toContain("60px");
        setCollapsed("host", true);
        expect(root.querySelector('[data-sticky-block="host"]')).toBeNull();
        setCollapsed("host", false);
        await vi.waitFor(() => expect(root.querySelector('[data-sticky-block="host"]')).not.toBeNull());
        changeStickyParents(false);
        expect(root.querySelector(".sticky-ancestor")).toBeNull();
        changeStickyParents(true);
        expect(root.querySelector('[data-sticky-block="host"]')).not.toBeNull();
        expect(root.querySelector<HTMLElement>('[data-sticky-block="source"]')!.style.top).toContain("60px");
      } finally { dispose(); }
    });
  }
}

it("a whole-block embed pins its one visible source-root bullet in the host's slot (D-20261030-EMB)", async () => {
  const source = leaf("target", "Embedded **root**", [leaf("kid", "Embedded child")]);
  const page: PageDto = { name: "Host", title: "Host", kind: "page", pre_block: null,
    blocks: [leaf("outer", "Outer", [leaf("host", "{{embed ((target))}}")]), source] };
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(() => ({ page: "Host", kind: "page", blocks: [source] })));
  loadFeed([page]);
  const root = document.createElement("div"); root.className = "main-content"; document.body.append(root);
  const dispose = render(() => <Block id="outer" />, root);
  try {
    await vi.waitFor(() => expect(root.querySelector('.embed-block [data-sticky-block="target"]')).not.toBeNull());
    expect(root.querySelectorAll(".sticky-ancestor")).toHaveLength(2);
    expect(root.querySelector('[data-sticky-block="host"]')).toBeNull();
    const pin = root.querySelector<HTMLElement>('[data-sticky-block="target"]')!;
    expect(pin.style.top).toContain("30px");
    expect(pin.querySelector("strong")?.textContent).toBe("root");
  } finally { dispose(); }
});

it("keeps pinning when a loaded ordinary parent changes into a page embed (GH #898)", async () => {
  const notify: (() => void)[] = [];
  vi.spyOn(realm, "newIntersectionObserver").mockImplementation((_realm, callback) => {
    let target: Element | undefined, live = true;
    const observer = { observe(el: Element) { target = el; }, disconnect() { live = false; } } as IntersectionObserver;
    notify.push(() => {
      if (!live || target?.closest(".ls-block")?.getAttribute("data-block-id") !== "host") return;
      // The host bullet is above its slot, while its embedded body still fills
      // the viewport. Deliver the geometry of the actual observed element.
      const bullet = target.hasAttribute("data-sticky-row");
      callback([{ target, isIntersecting: !bullet, boundingClientRect: { bottom: bullet ? -10 : 900 },
        rootBounds: { top: 30 } } as IntersectionObserverEntry], observer);
    });
    return observer;
  });
  const source = { id: "pages/Source.md", name: "Source", title: "Source", kind: "page" as const, pre_block: null,
    blocks: [leaf("source", "Embedded row")] };
  vi.spyOn(backend(), "getPage").mockResolvedValue(source);
  loadFeed([{ name: "Host", title: "Host", kind: "page", pre_block: null,
    blocks: [leaf("host", "Ordinary parent", [leaf("child", "Authored child")])] }, source]);
  const root = document.createElement("div"); root.className = "main-content"; document.body.append(root);
  const dispose = render(() => <Block id="host" />, root);
  try {
    setRaw("host", "{{embed [[Source]]}}");
    await vi.waitFor(() => expect(root.querySelector('.embed-block [data-block-id="source"]')).not.toBeNull());
    const button = root.querySelector<HTMLButtonElement>('[data-sticky-block="host"] button');
    expect(button).not.toBeNull();
    notify.forEach(run => run());
    expect(button!.hidden, "the changed host must pin before its embedded body leaves the viewport").toBe(false);
  } finally { dispose(); }
});
