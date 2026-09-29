import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { clearSeededFacets } from "../render/facets";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import type { BlockDto, PageDto } from "../types";

// Every block in this file is "referenced three times", so the badge renders
// without a backend round-trip.
vi.mock("../blockRefCounts", () => ({ blockRefCount: () => 3 }));

const { Block } = await import("./Block");

beforeAll(() => initParser());

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

function mount(page: PageDto) {
  loadSingle(page);
  clearSeededFacets();
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(
    () => <For each={pageByName(page.name)?.roots ?? []}>{(id) => <Block id={id} />}</For>,
    host,
  );
  return { host, dispose };
}

function page(blocks: BlockDto[]): PageDto {
  return {
    name: "Refs",
    title: "Refs",
    kind: "page",
    format: "md",
    pre_block: null,
    blocks,
  };
}

function block(id: string, raw: string): BlockDto {
  return { id, raw, collapsed: false, children: [] };
}

// GH #454. The badge is `float: right`, and a float rides the line box that is
// current where the browser meets it in the flow — so its POSITION IN THE DOM is
// what decides whether a wrapped block's badge sits on the first line or the
// last. jsdom applies no CSS layout and so cannot see the geometry itself
// (scripts/check-block-first-line-alignment.mjs measures that in a real
// browser); what it can pin, and what the geometry depends on, is the order.
describe("reference-count badge placement (GH #454)", () => {
  it("emits the badge as the first child of the block content, before any text", () => {
    const { host, dispose } = mount(page([block("b1", "Referenced prose that would wrap in a narrow column")]));
    try {
      const content = host.querySelector('[data-block-id="b1"] > .block-main .block-content');
      expect(content).not.toBeNull();
      const badge = content!.querySelector(".block-refs-count");
      expect(badge).not.toBeNull();
      expect(content!.firstElementChild).toBe(badge);
    } finally {
      dispose();
    }
  });

  it("keeps the badge first even when header chips (marker, priority) precede the text", () => {
    const { host, dispose } = mount(page([block("b2", "TODO [#A] A referenced task")]));
    try {
      const content = host.querySelector('[data-block-id="b2"] > .block-main .block-content');
      const badge = content!.querySelector(".block-refs-count");
      expect(badge).not.toBeNull();
      expect(content!.firstElementChild).toBe(badge);
      // The chips are still there, and still after it.
      expect(content!.querySelector(".block-marker")).not.toBeNull();
      expect(content!.querySelector(".block-priority")).not.toBeNull();
    } finally {
      dispose();
    }
  });

  it("keeps the badge's own behaviour: plain click opens the referrers panel", () => {
    const { host, dispose } = mount(page([block("b3", "Referenced block")]));
    try {
      const badge = host.querySelector<HTMLElement>('[data-block-id="b3"] .block-refs-count');
      expect(host.querySelector('[data-block-id="b3"] > .block-references')).toBeNull();
      badge!.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
      expect(host.querySelector('[data-block-id="b3"] > .block-references')).not.toBeNull();
    } finally {
      dispose();
    }
  });
});
