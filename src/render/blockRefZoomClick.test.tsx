import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetStore } from "../document";
import { AstBody } from "./body";
import { initParser } from "./parse";
import { setRefClickZoom } from "../copySettings";
import { paneRouter, resetPaneLayoutToSingle } from "../panes";

// GH #708: with "Click a block reference to zoom in" on, clicking a reference to
// a block on another page did nothing. The zoom needed the target block to be
// loaded in the current document, which a block on another page never is.

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  setRefClickZoom(false);
  resetStore();
  resetPaneLayoutToSingle();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

describe("block-reference click with ref-click zoom on (GH #708)", () => {
  it("zooms into a referenced block that lives on another, unloaded page", async () => {
    resetPaneLayoutToSingle();
    const id = "70800000-0000-4000-8000-000000000001";
    const target = { id, raw: "referenced text", collapsed: false, children: [], properties: [["id", id]] as [string, string][] };
    vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([{ page: "Other page", kind: "page" as const, blocks: [target] }]);
    setRefClickZoom(true);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <AstBody raw={`see ((${id}))`} />, host);
    try {
      await vi.waitFor(() => expect(host.querySelector(".block-ref")?.textContent?.trim()).toBe("referenced text"));
      (host.querySelector(".block-ref") as HTMLElement).click();
      const route = paneRouter("main").route();
      expect(route).toMatchObject({ kind: "page", name: "Other page", pageKind: "page", block: id });
    } finally {
      dispose();
    }
  });
});
