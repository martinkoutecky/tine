import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { initParser } from "../render/parse";
import { loadSingle, resetStore } from "../store";
import { setGraphMeta } from "../ui";
import type { BlockDto, PageDto } from "../types";
import { LiveRefGroup } from "./LiveRefGroup";
import { RefBlocks } from "./RefBlocks";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  setGraphMeta(null);
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function dto(): BlockDto {
  return { id: "macro-hit", raw: "{{rich}}", collapsed: false, children: [] };
}

function page(block: BlockDto): PageDto {
  return {
    name: "Source",
    title: "Source",
    kind: "page",
    pre_block: null,
    blocks: [block],
  };
}

function expectRich(root: HTMLElement): void {
  expect(root.querySelector(".lane-rich")?.textContent).toBe("Rich macro");
  expect(root.textContent).not.toContain("[:span");
}

describe("configured macro Hiccup on shared reference surfaces", () => {
  it("renders through an actual RefBlocks fallback", () => {
    setGraphMeta({ macros: { rich: '[:span.lane-rich "Rich macro"]' } } as never);
    const { root, dispose } = mount(() => <RefBlocks blocks={[dto()]} page="Source" pageKind="page" />);
    try {
      expectRich(root);
    } finally {
      dispose();
    }
  });

  it("renders through hydrated LiveRefGroup into the live Block route", async () => {
    setGraphMeta({ macros: { rich: '[:span.lane-rich "Rich macro"]' } } as never);
    const block = dto();
    loadSingle(page(block));
    const { root, dispose } = mount(() => (
      <LiveRefGroup page="Source" kind="page" blocks={[block]} surface="ref" />
    ));
    try {
      await expect.poll(() => root.querySelector(".lane-rich")?.textContent).toBe("Rich macro");
      expect(root.querySelector('[data-block-id="macro-hit"]')).not.toBeNull();
      expectRich(root);
    } finally {
      dispose();
    }
  });
});
