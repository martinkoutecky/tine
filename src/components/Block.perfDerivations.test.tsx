import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import { Block } from "./Block";

// Counters over the per-block derivations that used to run once in `Block` and
// again in `Rendered` (master 0350c00b6 / 023c986b0: bigLoad -8.8% / -9.4%).
// jsdom has no timing worth asserting, so the work itself is counted: rendering
// N plain blocks must cost one parent walk, one macro detection and one header
// gate evaluation per block, not two of each.
const counts = vi.hoisted(() => ({ depthOf: 0, macroExtents: 0, taskCheckbox: 0 }));

vi.mock("../document", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../document")>();
  return {
    ...actual,
    depthOf: (id: string) => {
      counts.depthOf++;
      return actual.depthOf(id);
    },
  };
});
vi.mock("../editor/queryMacro", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../editor/queryMacro")>();
  return {
    ...actual,
    queryMacroExtents: (text: string) => {
      counts.macroExtents++;
      return actual.queryMacroExtents(text);
    },
  };
});
vi.mock("../markers", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../markers")>();
  return {
    ...actual,
    taskCheckboxState: (marker: string | null | undefined) => {
      counts.taskCheckbox++;
      return actual.taskCheckboxState(marker as never);
    },
  };
});

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
  counts.depthOf = counts.macroExtents = counts.taskCheckbox = 0;
});

const N = 12;
const ids = Array.from({ length: N }, (_, i) => `b${i}`);

function node(id: string, raw: string): StoreNode {
  return { id, raw, collapsed: false, parent: null, page: "Perf", children: [] };
}

function page(roots: string[]): FeedPage {
  return { name: "Perf", kind: "page", title: "Perf", preBlock: null, roots, format: "md", readOnly: false, guide: false };
}

function mountPage(raws: string[]) {
  const rootIds = raws.map((_, i) => ids[i]);
  setDoc({
    byId: Object.fromEntries(rootIds.map((id, i) => [id, node(id, raws[i])])),
    pages: [page(rootIds)],
    feed: ["Perf"],
    loaded: true,
  });
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <For each={rootIds}>{(id) => <Block id={id} />}</For>, host);
  return { host, dispose };
}

describe("per-block derivations run once per block", () => {
  const plain = () => mountPage(ids.map((_, i) => `plain prose block ${i}`));

  it("walks the parent chain once per plain block", () => {
    const { host, dispose } = plain();
    try {
      expect(host.querySelectorAll(".ls-block").length).toBe(N);
      expect(counts.depthOf).toBeLessThanOrEqual(N);
    } finally {
      dispose();
    }
  });

  it("detects a standalone macro once per plain block", () => {
    const { dispose } = plain();
    try {
      expect(counts.macroExtents).toBeLessThanOrEqual(N);
    } finally {
      dispose();
    }
  });

  it("never enters the header-chip group of a block with no marker and no priority", () => {
    const { dispose } = plain();
    try {
      // Not even the checkbox decision is evaluated: one gate, not three.
      expect(counts.taskCheckbox).toBe(0);
    } finally {
      dispose();
    }
  });

  it("still renders every header chip of a task block", () => {
    const { host, dispose } = mountPage(["TODO [#A] write it\nSCHEDULED: <2026-01-05 Mon>\nowner:: Martin"]);
    try {
      expect(host.querySelector(".block-task-checkbox")).not.toBeNull();
      expect(host.querySelector(".block-marker")?.textContent).toBe("TODO");
      expect(host.querySelector(".block-priority")?.textContent).toBe("[#A]");
      expect(host.querySelector(".date-chip.scheduled")).not.toBeNull();
      expect(host.querySelector(".block-properties .prop-key")?.textContent).toBe("owner");
    } finally {
      dispose();
    }
  });

  it("re-derives the heading level when a block's text changes, in both faces of the row", () => {
    const { host, dispose } = mountPage(["plain"]);
    try {
      expect(host.querySelector(".block-content.heading")).toBeNull();
      expect(host.querySelector(".block-main")!.className).not.toMatch(/bullet-h/);
      setDoc("byId", "b0", "raw", "# now a heading");
      expect(host.querySelector(".block-content.heading.h1")).not.toBeNull();
      expect(host.querySelector(".block-main")!.className).toMatch(/bullet-h1/);
    } finally {
      dispose();
    }
  });
});
