import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import {
  clearTransientLayersForTest,
  dismissTopTransient,
  registerTransientLayer,
} from "../transientLayers";
import { QueryBuilder } from "./QueryBuilder";

function mountBuilder(dsl = "(and (task TODO))") {
  const host = document.createElement("div");
  document.body.append(host);
  const [source, setSource] = createSignal(dsl);
  const dispose = render(() => <QueryBuilder dsl={source} onChange={setSource} />, host);
  return { host, source, dispose };
}

afterEach(() => {
  clearTransientLayersForTest();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

beforeEach(() => {
  vi.spyOn(backend(), "queryFacets").mockResolvedValue([]);
});

describe("QueryBuilder transient ownership (post-GH #161)", () => {
  it("gives every popover family one Escape or Back rung above a lower owner without changing the DSL", () => {
    const { host, source, dispose } = mountBuilder();
    const original = source();
    const cases: Array<{ open: () => HTMLButtonElement; visible: string; reason: "escape" | "back" }> = [
      { open: () => host.querySelector<HTMLButtonElement>(".qb-chip")!, visible: ".qb-menu", reason: "escape" },
      { open: () => host.querySelector<HTMLButtonElement>(".qb-add")!, visible: ".qb-picker", reason: "back" },
      {
        open: () => [...host.querySelectorAll<HTMLButtonElement>(".qb-sort")]
          .find((button) => button.textContent?.trim() === "+ sort")!,
        visible: ".qb-sort-picker",
        reason: "escape",
      },
      {
        open: () => [...host.querySelectorAll<HTMLButtonElement>(".qb-sort")]
          .find((button) => button.textContent?.includes("summarize"))!,
        visible: ".qb-picker",
        reason: "back",
      },
    ];

    try {
      for (const testCase of cases) {
        const lower = vi.fn(() => true);
        const unregisterLower = registerTransientLayer({
          id: `query-builder-lower-${testCase.reason}-${testCase.visible}`,
          dismiss: lower,
        });
        testCase.open().click();
        expect(host.querySelector(testCase.visible)).not.toBeNull();

        expect(dismissTopTransient(testCase.reason)).toBe(true);
        expect(host.querySelector(testCase.visible)).toBeNull();
        expect(lower).not.toHaveBeenCalled();
        expect(source()).toBe(original);
        unregisterLower();
      }
    } finally {
      dispose();
    }
  });

  it("keeps two builder instances independent and reactivates an older visible peer", () => {
    const first = mountBuilder("(and (task TODO))");
    const second = mountBuilder("(and (priority A))");
    try {
      first.host.querySelector<HTMLButtonElement>(".qb-chip")!.click();
      second.host.querySelector<HTMLButtonElement>(".qb-chip")!.click();
      expect(first.host.querySelector(".qb-menu")).not.toBeNull();
      expect(second.host.querySelector(".qb-menu")).not.toBeNull();

      first.host.querySelector(".qb-menu")!.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
      expect(dismissTopTransient("escape")).toBe(true);
      expect(first.host.querySelector(".qb-menu")).toBeNull();
      expect(second.host.querySelector(".qb-menu")).not.toBeNull();

      expect(dismissTopTransient("back")).toBe(true);
      expect(second.host.querySelector(".qb-menu")).toBeNull();
    } finally {
      first.dispose();
      second.dispose();
    }
  });
});
